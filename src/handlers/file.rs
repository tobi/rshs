use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    body::Body,
    extract::State,
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};

use crate::server::AppState;
use crate::utils::time::format_modified;

pub async fn handle(State(state): State<Arc<AppState>>, req: axum::extract::Request) -> Response {
    let request_path = req.uri().path().to_owned();

    let rel_path = request_path.trim_start_matches('/');
    let fs_path = state.root_dir.join(rel_path);

    tracing::debug!(method = %req.method(), path = %request_path, "incoming request");

    let fs_path = match tokio::fs::canonicalize(&fs_path).await {
        Ok(p) => p,
        Err(_) => {
            tracing::debug!(method = %req.method(), path = %request_path, status = 404, "path not found");
            return StatusCode::NOT_FOUND.into_response();
        }
    };

    let root_canonical = &state.root_canonical;
    if !fs_path.starts_with(root_canonical.as_path()) {
        tracing::warn!(
            method = %req.method(),
            path = %request_path,
            status = 404,
            "path traversal blocked",
        );
        return StatusCode::NOT_FOUND.into_response();
    }

    match *req.method() {
        Method::GET | Method::HEAD => {
            if match tokio::fs::metadata(&fs_path).await {
                Ok(m) => m.is_dir(),
                Err(_) => false,
            } {
                let (html, entry_count) = generate_dir_listing(&fs_path, &request_path).await;
                tracing::debug!(
                    method = %req.method(),
                    path = %request_path,
                    status = 200,
                    entry_count = entry_count,
                    "directory listing"
                );
                let body_len = html.len();
                let mut resp = Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "text/html; charset=utf-8");
                if *req.method() == Method::HEAD {
                    resp = resp.header("content-length", body_len.to_string());
                    resp.body(Body::empty()).unwrap()
                } else {
                    resp.body(Body::from(html)).unwrap()
                }
            } else {
                match tokio::fs::read(&fs_path).await {
                    Ok(data) => {
                        let data_len = data.len();
                        let mime = mime_guess::from_path(&fs_path).first_or_octet_stream();
                        tracing::debug!(
                            method = %req.method(),
                            path = %request_path,
                            status = 200,
                            mime = %mime.essence_str(),
                            size = data_len,
                            "file served"
                        );
                        let mut resp = Response::builder()
                            .status(StatusCode::OK)
                            .header("content-type", mime.as_ref());
                        if *req.method() == Method::HEAD {
                            resp = resp.header("content-length", data_len.to_string());
                            resp.body(Body::empty()).unwrap()
                        } else {
                            resp.body(Body::from(data)).unwrap()
                        }
                    }
                    Err(_) => {
                        tracing::error!(
                            method = %req.method(),
                            path = %request_path,
                            status = 500,
                            "failed to read file",
                        );
                        StatusCode::INTERNAL_SERVER_ERROR.into_response()
                    }
                }
            }
        }
        _ => {
            tracing::debug!(
                method = %req.method(),
                path = %request_path,
                status = 405,
                "method not allowed",
            );
            StatusCode::METHOD_NOT_ALLOWED.into_response()
        }
    }
}

async fn generate_dir_listing(dir_path: &std::path::Path, request_path: &str) -> (String, usize) {
    let mut read_dir = match tokio::fs::read_dir(dir_path).await {
        Ok(rd) => rd,
        Err(_) => {
            return ("<!DOCTYPE html>\n<html>\n<head>\n<title>Error</title>\n</head>\n<body>\n<h1>Cannot read directory</h1>\n</body>\n</html>\n".to_string(), 0);
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
    html.push_str("<!DOCTYPE html>\n<html>\n<head>\n");
    html.push_str(&format!("<title>Index of {request_path}</title>\n"));
    html.push_str("<meta charset=\"utf-8\">\n</head>\n<body>\n");
    html.push_str(&format!("<h1>Index of {request_path}</h1>\n<hr>\n<pre>"));
    if request_path != "/" {
        html.push_str("<a href=\"../\">../</a>\n");
    }

    for entry in &entries {
        let disp = display(entry);
        let size_str = size_label(entry);
        let date_str = format_modified(entry.3);
        let pad1 = name_col.saturating_sub(disp.len());

        let anchor = if entry.1 {
            format!("<a href=\"{}/\">{}/</a>", entry.0, entry.0)
        } else {
            format!("<a href=\"{}\">{}</a>", entry.0, entry.0)
        };

        html.push_str(&format!(
            "{anchor}{:pad1$}{date_str}    {:>max_size_len$}\n",
            "", size_str
        ));
    }

    let entry_count = entries.len();
    html.push_str("</pre>\n<hr>\n</body>\n</html>\n");
    (html, entry_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn test_generate_dir_listing_structure() {
        let dir = tempfile::TempDir::new().expect("failed to create temp dir");
        let mut f = std::fs::File::create(dir.path().join("hello.txt")).unwrap();
        f.write_all(b"hello").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let (html, count) = generate_dir_listing(dir.path(), "/").await;

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<title>Index of /</title>"));
        assert!(html.contains("<pre>"));
        assert!(html.contains("hello.txt"));
        assert!(html.contains("subdir/"));
        assert!(!html.contains("../"));
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_generate_dir_listing_subdir_has_parent_link() {
        let dir = tempfile::TempDir::new().expect("failed to create temp dir");
        let mut f = std::fs::File::create(dir.path().join("data.bin")).unwrap();
        f.write_all(b"bin").unwrap();

        let (html, count) = generate_dir_listing(dir.path(), "/sub/").await;

        assert!(html.contains("Index of /sub/"));
        assert!(html.contains("../"));
        assert!(html.contains("data.bin"));
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_generate_dir_listing_empty_dir() {
        let dir = tempfile::TempDir::new().expect("failed to create temp dir");

        let (html, count) = generate_dir_listing(dir.path(), "/empty/").await;

        assert!(html.contains("Index of /empty/"));
        assert!(html.contains("../"));
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_generate_dir_listing_dirs_before_files() {
        let dir = tempfile::TempDir::new().expect("failed to create temp dir");
        std::fs::create_dir(dir.path().join("zzz_dir")).unwrap();
        let mut f = std::fs::File::create(dir.path().join("aaa_file.txt")).unwrap();
        f.write_all(b"x").unwrap();

        let (html, count) = generate_dir_listing(dir.path(), "/").await;

        assert_eq!(count, 2);

        let zzz_pos = html.find("zzz_dir").unwrap();
        let aaa_pos = html.find("aaa_file").unwrap();
        assert!(zzz_pos < aaa_pos, "directories should appear before files");
    }
}
