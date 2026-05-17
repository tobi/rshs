use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::TryStreamExt;
use tokio::io::AsyncWriteExt;
use tokio_util::io::{ReaderStream, StreamReader};

use crate::server::AppState;
use crate::utils::{path, time::format_rfc850};

pub use axum::http::Method;

// ---------------------------------------------------------------------------
// GET / HEAD
// ---------------------------------------------------------------------------

pub async fn handle_get_head(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_path = req.uri().path().to_owned();

    let fs_path = match state.resolve_existing(&request_path).await {
        Some(p) => p,
        None => {
            tracing::debug!("path resolution failed");
            return StatusCode::NOT_FOUND.into_response();
        }
    };

    do_get_or_head(fs_path, request_path, req.method()).await
}

async fn do_get_or_head(fs_path: PathBuf, request_path: String, method: &Method) -> Response {
    let meta = match tokio::fs::metadata(&fs_path).await {
        Ok(m) => m,
        Err(_) => {
            tracing::debug!("metadata failed");
            return StatusCode::NOT_FOUND.into_response();
        }
    };

    if meta.is_dir() {
        let (html, entry_count) = generate_dir_listing(&fs_path, &request_path).await;
        tracing::debug!(entry_count = entry_count, "directory listing");
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/html; charset=utf-8")
            .header("content-length", html.len());
        if method == Method::HEAD {
            return resp.body(Body::empty()).unwrap();
        }
        resp.body(Body::from(html)).unwrap()
    } else {
        let file_size = meta.len();
        let mime = mime_guess::from_path(&fs_path).first_or_octet_stream();
        tracing::debug!(mime = %mime.essence_str(), size = file_size, "file served");
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header("content-type", mime.as_ref())
            .header("content-length", file_size);
        if method == Method::HEAD {
            return resp.body(Body::empty()).unwrap();
        }
        match tokio::fs::File::open(&fs_path).await {
            Ok(file) => {
                let stream = ReaderStream::new(file);
                resp.body(Body::from_stream(stream)).unwrap()
            }
            Err(e) => {
                tracing::error!(error = %e, "open failed");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

async fn generate_dir_listing(dir_path: &Path, request_path: &str) -> (String, usize) {
    let mut read_dir = match tokio::fs::read_dir(dir_path).await {
        Ok(rd) => rd,
        Err(_) => {
            return (
                "<!DOCTYPE html><html><head><title>Error</title></head><body><h1>Cannot read directory</h1></body></html>"
                    .to_string(),
                0,
            );
        }
    };

    let mut entries: Vec<(String, bool, u64, SystemTime)> = Vec::new();
    loop {
        let entry = match read_dir.next_entry().await {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
        let metadata = entry.metadata().await.ok();
        let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .unwrap_or(UNIX_EPOCH);
        entries.push((name, is_dir, size, modified));
    }

    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let display = |e: &(String, bool, u64, SystemTime)| {
        if e.1 {
            format!("{}/", e.0)
        } else {
            e.0.clone()
        }
    };

    let size_label = |e: &(String, bool, u64, SystemTime)| {
        if e.1 {
            "-".to_string()
        } else {
            e.2.to_string()
        }
    };

    let max_name_len = entries.iter().map(|e| display(e).len()).max().unwrap_or(0);
    let max_size_len = entries
        .iter()
        .map(|e| size_label(e).len())
        .max()
        .unwrap_or(0);
    let name_col = max_name_len + 20;

    let mut html = String::new();
    html.push_str("<!DOCTYPE html><html><head>");
    html.push_str(&format!("<title>Index of {request_path}</title>"));
    html.push_str("<meta charset=\"utf-8\"></head><body>");
    html.push_str(&format!("<h1>Index of {request_path}</h1><hr><pre>"));
    if request_path != "/" {
        html.push_str("<a href=\"../\">../</a>");
    }

    for entry in &entries {
        let disp = display(entry);
        let size_str = size_label(entry);
        let date_str = format_rfc850(entry.3);
        let pad1 = name_col.saturating_sub(disp.len());

        let anchor = if entry.1 {
            format!("<a href=\"{}/\">{}/</a>", entry.0, entry.0)
        } else {
            format!("<a href=\"{}\">{}</a>", entry.0, entry.0)
        };

        html.push_str(&format!(
            "{anchor}{:pad1$}{date_str}    {:>max_size_len$}",
            "", size_str
        ));
    }

    let entry_count = entries.len();
    html.push_str("</pre><hr></body></html>");
    (html, entry_count)
}

// ---------------------------------------------------------------------------
// PUT
// ---------------------------------------------------------------------------

pub async fn handle_put(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_path = req.uri().path().to_owned();

    // PUT MUST NOT create intermediate collections (RFC 4918 §9.6)
    let target = match state.resolve_and_guard(&request_path, false).await {
        Ok(t) => t,
        Err(path::ResolveTargetError::InvalidPath) => {
            tracing::debug!("path resolution failed");
            return StatusCode::BAD_REQUEST.into_response();
        }
        Err(path::ResolveTargetError::ParentCanonicalizeFailed(_)) => {
            tracing::debug!("parent directory does not exist for PUT");
            return StatusCode::CONFLICT.into_response();
        }
        Err(path::ResolveTargetError::TraversalBlocked) => {
            tracing::warn!(path = %request_path, "path traversal blocked in PUT");
            return StatusCode::FORBIDDEN.into_response();
        }
    };

    let existed = match tokio::fs::metadata(&target).await {
        Ok(m) => m.is_file(),
        Err(_) => false,
    };

    let mut file = match tokio::fs::File::create(&target).await {
        Ok(f) => f,
        Err(e) => {
            tracing::error!(
                error = %e, path = %target.display(), "failed to create file for PUT"
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let body = req.into_body();
    let stream = body.into_data_stream().map_err(std::io::Error::other);
    let mut reader = StreamReader::new(stream);

    let bytes_written = match tokio::io::copy(&mut reader, &mut file).await {
        Ok(n) => n,
        Err(e) => {
            tracing::error!(
                error = %e, path = %target.display(), "error writing PUT body"
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if let Err(e) = file.flush().await {
        tracing::error!(error = %e, path = %target.display(), "error flushing PUT file");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    tracing::debug!(
        path = %target.display(), size = bytes_written, existed = existed, "PUT completed"
    );

    if existed {
        StatusCode::OK.into_response()
    } else {
        StatusCode::CREATED.into_response()
    }
}

// ---------------------------------------------------------------------------
// DELETE
// ---------------------------------------------------------------------------

pub async fn handle_delete(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_path = req.uri().path().to_owned();

    let fs_path = match state.resolve_existing(&request_path).await {
        Some(p) => p,
        None => {
            tracing::debug!("path resolution failed for DELETE");
            return StatusCode::NOT_FOUND.into_response();
        }
    };

    let meta = match tokio::fs::metadata(&fs_path).await {
        Ok(m) => m,
        Err(_) => {
            tracing::debug!("metadata failed for DELETE");
            return StatusCode::NOT_FOUND.into_response();
        }
    };

    if meta.is_dir() {
        if fs_path == state.root_canonical {
            tracing::debug!("DELETE rejected: root directory");
            return StatusCode::BAD_REQUEST.into_response();
        }
        match tokio::fs::remove_dir_all(&fs_path).await {
            Ok(()) => {
                tracing::debug!(path = %fs_path.display(), "DELETE directory completed");
                StatusCode::NO_CONTENT.into_response()
            }
            Err(e) => {
                tracing::error!(
                    error = %e, path = %fs_path.display(), "failed to remove directory for DELETE"
                );
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    } else {
        match tokio::fs::remove_file(&fs_path).await {
            Ok(()) => {
                tracing::debug!(path = %fs_path.display(), "DELETE completed");
                StatusCode::NO_CONTENT.into_response()
            }
            Err(e) => {
                tracing::error!(
                    error = %e, path = %fs_path.display(), "failed to remove file for DELETE"
                );
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// OPTIONS
// ---------------------------------------------------------------------------

pub async fn handle_options() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(
            "allow",
            "GET, HEAD, OPTIONS, PUT, DELETE, PROPFIND, MKCOL, COPY, MOVE, PROPPATCH, LOCK, UNLOCK",
        )
        .header("dav", "1,2")
        .header("ms-author-via", "DAV")
        .header("content-length", "0")
        .body(Body::empty())
        .unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{Router, body::Body, extract::Request};
    use tower::ServiceExt;

    use crate::{AppState, AuthConfig};

    #[tokio::test]
    async fn test_generate_dir_listing_structure() {
        let dir = tempfile::TempDir::new().unwrap();
        use std::io::Write;
        let mut f = std::fs::File::create(dir.path().join("hello.txt")).unwrap();
        f.write_all(b"hello").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let (html, count) = super::generate_dir_listing(dir.path(), "/").await;

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<title>Index of /</title>"));
        assert!(!html.contains("../"));
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_generate_dir_listing_subdir_has_parent_link() {
        let dir = tempfile::TempDir::new().unwrap();
        use std::io::Write;
        let mut f = std::fs::File::create(dir.path().join("data.bin")).unwrap();
        f.write_all(b"bin").unwrap();

        let (html, count) = super::generate_dir_listing(dir.path(), "/sub/").await;

        assert!(html.contains("Index of /sub/"));
        assert!(html.contains("../"));
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_generate_dir_listing_empty_dir() {
        let dir = tempfile::TempDir::new().unwrap();

        let (html, count) = super::generate_dir_listing(dir.path(), "/empty/").await;

        assert!(html.contains("Index of /empty/"));
        assert!(html.contains("../"));
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_generate_dir_listing_dirs_before_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("zzz_dir")).unwrap();
        use std::io::Write;
        let mut f = std::fs::File::create(dir.path().join("aaa_file.txt")).unwrap();
        f.write_all(b"x").unwrap();

        let (html, count) = super::generate_dir_listing(dir.path(), "/").await;

        assert_eq!(count, 2);
        let zzz_pos = html.find("zzz_dir").unwrap();
        let aaa_pos = html.find("aaa_file").unwrap();
        assert!(zzz_pos < aaa_pos, "directories should appear before files");
    }

    // -- PUT tests -----------------------------------------------------------

    fn make_app_put(dir: &tempfile::TempDir) -> Router {
        use std::collections::HashMap;

        use tokio::sync::RwLock;

        let root = dir.path().to_path_buf();
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        Router::new()
            .route("/", axum::routing::put(super::handle_put))
            .route("/{*path}", axum::routing::put(super::handle_put))
            .with_state(Arc::new(AppState {
                root_dir: root.clone(),
                root_canonical: canonical,
                auth_config: Arc::new(AuthConfig::new()),
                dead_props: Arc::new(RwLock::new(HashMap::new())),
                locks: Arc::new(RwLock::new(HashMap::new())),
            }))
    }

    #[tokio::test]
    async fn test_put_creates_new_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_put(&dir);

        let req = Request::builder()
            .method(axum::http::Method::PUT)
            .uri("/newfile.txt")
            .header("content-type", "text/plain")
            .body(Body::from("hello put"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CREATED);

        let content = std::fs::read_to_string(dir.path().join("newfile.txt")).unwrap();
        assert_eq!(content, "hello put");
    }

    #[tokio::test]
    async fn test_put_overwrites_existing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("existing.txt"), b"old content").unwrap();
        let app = make_app_put(&dir);

        let req = Request::builder()
            .method(axum::http::Method::PUT)
            .uri("/existing.txt")
            .body(Body::from("new content"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let content = std::fs::read_to_string(dir.path().join("existing.txt")).unwrap();
        assert_eq!(content, "new content");
    }

    #[tokio::test]
    async fn test_put_empty_body() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_put(&dir);

        let req = Request::builder()
            .method(axum::http::Method::PUT)
            .uri("/empty.txt")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CREATED);

        let content = std::fs::read_to_string(dir.path().join("empty.txt")).unwrap();
        assert!(content.is_empty());
    }

    #[tokio::test]
    async fn test_put_rejects_missing_parent() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_put(&dir);

        let req = Request::builder()
            .method(axum::http::Method::PUT)
            .uri("/a/b/c/file.txt")
            .body(Body::from("nested"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // RFC 4918 §9.6: PUT MUST NOT create intermediate collections
        assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn test_put_rejects_root_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_put(&dir);

        let req = Request::builder()
            .method(axum::http::Method::PUT)
            .uri("/")
            .body(Body::from("bad"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_put_rejects_dir_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_put(&dir);

        let req = Request::builder()
            .method(axum::http::Method::PUT)
            .uri("/subdir/")
            .body(Body::from("bad"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_put_rejects_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_put(&dir);

        let req = Request::builder()
            .method(axum::http::Method::PUT)
            .uri("/../outside.txt")
            .body(Body::from("bad"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    // -- DELETE tests --------------------------------------------------------

    fn make_app_delete(dir: &tempfile::TempDir) -> Router {
        use std::collections::HashMap;

        use tokio::sync::RwLock;

        let root = dir.path().to_path_buf();
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        Router::new()
            .route("/", axum::routing::delete(super::handle_delete))
            .route("/{*path}", axum::routing::delete(super::handle_delete))
            .with_state(Arc::new(AppState {
                root_dir: root.clone(),
                root_canonical: canonical,
                auth_config: Arc::new(AuthConfig::new()),
                dead_props: Arc::new(RwLock::new(HashMap::new())),
                locks: Arc::new(RwLock::new(HashMap::new())),
            }))
    }

    #[tokio::test]
    async fn test_delete_existing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("remove_me.txt"), b"data").unwrap();
        let app = make_app_delete(&dir);

        let req = Request::builder()
            .method(axum::http::Method::DELETE)
            .uri("/remove_me.txt")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NO_CONTENT);
        assert!(!dir.path().join("remove_me.txt").exists());
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_delete(&dir);

        let req = Request::builder()
            .method(axum::http::Method::DELETE)
            .uri("/ghost.txt")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let subdir = dir.path().join("mydir");
        std::fs::create_dir(&subdir).unwrap();
        std::fs::write(subdir.join("inner.txt"), b"inside").unwrap();
        assert!(subdir.exists());
        let app = make_app_delete(&dir);

        let req = Request::builder()
            .method(axum::http::Method::DELETE)
            .uri("/mydir")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NO_CONTENT);
        assert!(!subdir.exists());
    }

    #[tokio::test]
    async fn test_delete_root_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_delete(&dir);

        let req = Request::builder()
            .method(axum::http::Method::DELETE)
            .uri("/")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_delete_dir_trailing_slash() {
        let dir = tempfile::TempDir::new().unwrap();
        let subdir = dir.path().join("mydir");
        std::fs::create_dir(&subdir).unwrap();
        assert!(subdir.exists());
        let app = make_app_delete(&dir);

        let req = Request::builder()
            .method(axum::http::Method::DELETE)
            .uri("/mydir/")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NO_CONTENT);
        assert!(!subdir.exists());
    }

    #[tokio::test]
    async fn test_delete_rejects_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = make_app_delete(&dir);

        let req = Request::builder()
            .method(axum::http::Method::DELETE)
            .uri("/../outside.txt")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    // -- OPTIONS tests -------------------------------------------------------

    #[tokio::test]
    async fn test_options_returns_ok() {
        let resp = super::handle_options().await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let allow = resp.headers().get("allow").unwrap().to_str().unwrap();
        assert!(allow.contains("GET"));
        assert!(allow.contains("PUT"));
        assert!(allow.contains("DELETE"));
        assert!(allow.contains("PROPFIND"));
        assert!(allow.contains("MKCOL"));
        assert_eq!(resp.headers().get("dav").unwrap().to_str().unwrap(), "1,2");
        assert_eq!(
            resp.headers()
                .get("ms-author-via")
                .unwrap()
                .to_str()
                .unwrap(),
            "DAV"
        );
    }

    #[tokio::test]
    async fn test_options_has_content_length_zero() {
        let resp = super::handle_options().await;
        let cl = resp
            .headers()
            .get("content-length")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(cl, "0");
    }

    #[tokio::test]
    async fn test_options_body_empty() {
        let resp = super::handle_options().await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(body.is_empty());
    }
}
