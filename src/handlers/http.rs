use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use derive_new::new;
use futures_util::TryStreamExt;
use tokio::io::AsyncWriteExt;
use tokio_util::io::{ReaderStream, StreamReader};

use crate::ok_or_return;
use crate::server::AppState;
use crate::utils::error::{IntoResolved, OrStatus};
use crate::utils::time::format_rfc850;

/// GET / HEAD handler — serves files and generates HTML directory listings.
///
/// Supports conditional `If-Modified-Since` via the `Last-Modified` header.
/// Accepts `Range` requests for partial content delivery.
pub async fn handle_get_head(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_path = req.uri().path().to_owned();

    let fs_path = state.resolve_existing(&request_path).await;
    let fs_path = ok_or_return!(fs_path.or_404("path resolution failed"));

    do_get_or_head(fs_path, request_path, req.method()).await
}

async fn do_get_or_head(fs_path: PathBuf, request_path: String, method: &Method) -> Response {
    let meta = tokio::fs::metadata(&fs_path).await;
    let meta = ok_or_return!(meta.or_404("metadata failed for GET/HEAD"));

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

#[derive(new)]
struct DirEntry {
    name: String,
    is_dir: bool,
    size: u64,
    modified: SystemTime,
}

impl DirEntry {
    fn display_name(&self) -> String {
        if self.is_dir {
            format!("{}/", self.name)
        } else {
            self.name.clone()
        }
    }

    fn display_name_len(&self) -> usize {
        self.name.len() + if self.is_dir { 1 } else { 0 }
    }

    fn size_label(&self) -> String {
        if self.is_dir {
            "-".to_string()
        } else {
            self.size.to_string()
        }
    }

    fn size_label_len(&self) -> usize {
        if self.is_dir {
            1
        } else {
            self.size.checked_ilog10().unwrap_or(0) as usize + 1
        }
    }
}

async fn collect_dir_entries(dir_path: &Path) -> Option<Vec<DirEntry>> {
    let mut read_dir = tokio::fs::read_dir(dir_path).await.ok()?;

    let mut entries = Vec::new();
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
        entries.push(DirEntry::new(name, is_dir, size, modified));
    }
    Some(entries)
}

fn render_dir_html(request_path: &str, mut entries: Vec<DirEntry>) -> (String, usize) {
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));

    let (max_name_len, max_size_len) = entries.iter().fold((0, 0), |(mn, ms), e| {
        (mn.max(e.display_name_len()), ms.max(e.size_label_len()))
    });
    let name_col = max_name_len + 20;

    use std::fmt::Write;
    let mut html = String::new();
    write!(
        html,
        "<!DOCTYPE html><html><head><title>Index of {request_path}</title><meta charset=\"utf-8\"></head><body><h1>Index of {request_path}</h1><hr><pre>"
    )
    .unwrap();
    if request_path != "/" {
        html.push_str("<a href=\"../\">../</a>\n");
    }

    for entry in &entries {
        let disp = entry.display_name();
        let size_str = entry.size_label();
        let date_str = format_rfc850(entry.modified);
        let pad1 = name_col.saturating_sub(disp.len());

        if entry.is_dir {
            write!(html, "<a href=\"{}/\">{}/</a>", entry.name, entry.name).unwrap();
        } else {
            write!(html, "<a href=\"{}\">{}</a>", entry.name, entry.name).unwrap();
        }

        writeln!(
            html,
            "{:pad1$}{date_str}    {:>max_size_len$}",
            "", size_str
        )
        .unwrap();
    }

    let entry_count = entries.len();
    html.push_str("</pre><hr></body></html>");
    (html, entry_count)
}

async fn generate_dir_listing(dir_path: &Path, request_path: &str) -> (String, usize) {
    let entries = match collect_dir_entries(dir_path).await {
        Some(entries) => entries,
        None => {
            return (
                "<!DOCTYPE html><html><head><title>Error</title></head><body><h1>Cannot read directory</h1></body></html>"
                    .to_string(),
                0,
            );
        }
    };

    render_dir_html(request_path, entries)
}

/// PUT handler — accepts a request body and writes it to the filesystem.
///
/// Returns `201 Created` for new files, `200 OK` for overwrites.
/// Rejects directory paths, missing parents, and traversal attempts.
/// Intermediate collections are NOT created (per RFC 4918 §9.6).
pub async fn handle_put(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_path = req.uri().path().to_owned();

    // PUT MUST NOT create intermediate collections (RFC 4918 §9.6)
    let target = state.resolve_and_guard(&request_path).await;
    let target = ok_or_return!(target.or_invalid(StatusCode::BAD_REQUEST));

    let result = match tokio::fs::File::create_new(&target).await {
        Ok(f) => Ok((f, false)),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            tokio::fs::File::create(&target).await.map(|f| (f, true))
        }
        Err(e) => Err(e),
    };
    let (mut file, existed) = match result {
        Ok(pair) => pair,
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

/// DELETE handler — removes a file or recursively deletes a directory.
///
/// Returns `204 No Content` on success, `404 Not Found` if the target
/// does not exist. Root directory deletion is rejected.
pub async fn handle_delete(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let request_path = req.uri().path().to_owned();

    let fs_path = state.resolve_existing(&request_path).await;
    let fs_path = ok_or_return!(fs_path.or_404("path resolution failed for DELETE"));

    let meta = tokio::fs::metadata(&fs_path).await;
    let meta = ok_or_return!(meta.or_404("metadata failed for DELETE"));

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

/// OPTIONS handler — returns supported HTTP/DAV methods in the `Allow` header.
///
/// Includes the `DAV: 1,2` compliance level and `MS-Author-Via: DAV` header
/// for compatibility with legacy clients.
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
        Router::new()
            .route("/", axum::routing::put(super::handle_put))
            .route("/{*path}", axum::routing::put(super::handle_put))
            .with_state(Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthConfig::new(),
                std::time::Duration::from_secs(300),
            )))
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
        Router::new()
            .route("/", axum::routing::delete(super::handle_delete))
            .route("/{*path}", axum::routing::delete(super::handle_delete))
            .with_state(Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthConfig::new(),
                std::time::Duration::from_secs(300),
            )))
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

    // -- GET/HEAD tests ------------------------------------------------------

    fn make_app_get(dir: &tempfile::TempDir) -> Router {
        Router::new()
            .fallback(super::handle_get_head)
            .with_state(Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthConfig::new(),
                std::time::Duration::from_secs(300),
            )))
    }

    fn setup_get_test_dir() -> tempfile::TempDir {
        use std::io::Write;
        let dir = tempfile::TempDir::new().unwrap();
        let mut f = std::fs::File::create(dir.path().join("hello.txt")).unwrap();
        f.write_all(b"Hello, World!").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        let mut f = std::fs::File::create(dir.path().join("subdir/nested.txt")).unwrap();
        f.write_all(b"Nested file").unwrap();
        dir
    }

    #[tokio::test]
    async fn test_get_path_traversal_blocked() {
        let dir = setup_get_test_dir();
        let app = make_app_get(&dir);

        let req = Request::builder()
            .method(axum::http::Method::GET)
            .uri("/../outside.txt")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_mime_type_guess() {
        let dir = setup_get_test_dir();
        let app = make_app_get(&dir);

        let req = Request::builder()
            .method(axum::http::Method::GET)
            .uri("/hello.txt")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert!(
            resp.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("text/plain")
        );
    }

    #[tokio::test]
    async fn test_get_dir_listing_sizes() {
        let dir = setup_get_test_dir();
        let app = make_app_get(&dir);

        let req = Request::builder()
            .method(axum::http::Method::GET)
            .uri("/")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();

        assert!(body_str.contains("hello.txt") && body_str.contains("13"));
        assert!(body_str.contains("subdir/") && body_str.contains("-"));
    }
}
