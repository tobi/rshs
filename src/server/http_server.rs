use std::fs;
use std::path::PathBuf;

use actix_web::{HttpRequest, HttpResponse, http::header, web};

pub async fn handle(req: HttpRequest, root_dir: web::Data<PathBuf>) -> HttpResponse {
    let request_path = req.path();

    let rel_path = request_path.trim_start_matches('/');
    let fs_path = root_dir.join(rel_path);

    tracing::debug!(method = %req.method(), path = request_path, "incoming request");

    let fs_path = match fs_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            tracing::debug!(method = %req.method(), path = request_path, status = 404, err = "not_found");
            return HttpResponse::NotFound().finish();
        }
    };

    let root_canonical = root_dir
        .canonicalize()
        .unwrap_or_else(|_| (**root_dir).clone());
    if !fs_path.starts_with(&root_canonical) {
        tracing::warn!(
            method = %req.method(),
            path = request_path,
            status = 404,
            err = "path_traversal",
        );
        return HttpResponse::NotFound().finish();
    }

    match *req.method() {
        actix_web::http::Method::GET | actix_web::http::Method::HEAD => {
            if fs_path.is_dir() {
                let (html, entry_count) = generate_dir_listing(&fs_path, request_path);
                tracing::debug!(
                    method = %req.method(),
                    path = request_path,
                    status = 200,
                    entry_count = entry_count,
                    "directory listing"
                );
                let body_len = html.len();
                let mut resp = HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(html);
                if *req.method() == actix_web::http::Method::HEAD {
                    resp = HttpResponse::Ok()
                        .content_type("text/html; charset=utf-8")
                        .insert_header((header::CONTENT_LENGTH, body_len))
                        .finish();
                }
                resp
            } else {
                match fs::read(&fs_path) {
                    Ok(data) => {
                        let data_len = data.len();
                        let mime = mime_guess::from_path(&fs_path).first_or_octet_stream();
                        tracing::debug!(
                            method = %req.method(),
                            path = request_path,
                            status = 200,
                            mime = %mime.essence_str(),
                            size = data_len,
                            "file served"
                        );
                        let mut resp = HttpResponse::Ok();
                        resp.content_type(mime.as_ref());

                        if *req.method() == actix_web::http::Method::HEAD {
                            resp.insert_header((header::CONTENT_LENGTH, data_len));
                            resp.finish()
                        } else {
                            resp.body(data)
                        }
                    }
                    Err(_) => {
                        tracing::error!(
                            method = %req.method(),
                            path = request_path,
                            status = 500,
                            err = "read_file_failed",
                        );
                        HttpResponse::InternalServerError().finish()
                    }
                }
            }
        }
        _ => {
            tracing::debug!(
                method = %req.method(),
                path = request_path,
                status = 405,
                err = "method_not_allowed",
            );
            HttpResponse::MethodNotAllowed().finish()
        }
    }
}

fn generate_dir_listing(dir_path: &std::path::Path, request_path: &str) -> (String, usize) {
    let entries: Vec<_> = match fs::read_dir(dir_path) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => {
            return ("<!DOCTYPE html>\n<html>\n<head>\n<title>Error</title>\n</head>\n<body>\n<h1>Cannot read directory</h1>\n</body>\n</html>\n".to_string(), 0);
        }
    };

    let mut entries: Vec<_> = entries
        .into_iter()
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            (name, is_dir, size)
        })
        .collect();

    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut html = String::new();
    html.push_str("<!DOCTYPE html>\n<html>\n<head>\n");
    html.push_str(&format!(
        "<title>Directory listing for {request_path}</title>\n"
    ));
    html.push_str("<meta charset=\"utf-8\">\n</head>\n<body>\n");
    html.push_str(&format!(
        "<h1>Directory listing for {request_path}</h1>\n<hr>\n<ul>\n"
    ));

    if request_path != "/" {
        html.push_str("<li><a href=\"../\">../</a></li>\n");
    }

    for (name, is_dir, size) in &entries {
        let encoded_name = name;
        if *is_dir {
            html.push_str(&format!(
                "<li><a href=\"{encoded_name}/\">{encoded_name}/</a></li>\n"
            ));
        } else {
            html.push_str(&format!(
                "<li><a href=\"{encoded_name}\">{encoded_name}</a> ({})</li>\n",
                format_size(*size)
            ));
        }
    }

    let entry_count = entries.len();
    html.push_str("</ul>\n<hr>\n</body>\n</html>\n");
    (html, entry_count)
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit_idx])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(42), "42 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn test_format_size_kb() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(10240), "10.0 KB");
    }

    #[test]
    fn test_format_size_mb() {
        assert_eq!(format_size(1048576), "1.0 MB");
        assert_eq!(format_size(1572864), "1.5 MB");
    }

    #[test]
    fn test_format_size_gb() {
        assert_eq!(format_size(1073741824), "1.0 GB");
    }

    #[test]
    fn test_generate_dir_listing_structure() {
        let dir = tempfile::TempDir::new().expect("failed to create temp dir");
        let mut f = std::fs::File::create(dir.path().join("hello.txt")).unwrap();
        f.write_all(b"hello").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let (html, count) = generate_dir_listing(dir.path(), "/");

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<title>Directory listing for /</title>"));
        assert!(html.contains("hello.txt"));
        assert!(html.contains("subdir/"));
        assert!(!html.contains("../"));
        assert_eq!(count, 2);
    }

    #[test]
    fn test_generate_dir_listing_subdir_has_parent_link() {
        let dir = tempfile::TempDir::new().expect("failed to create temp dir");
        let mut f = std::fs::File::create(dir.path().join("data.bin")).unwrap();
        f.write_all(b"bin").unwrap();

        let (html, count) = generate_dir_listing(dir.path(), "/sub/");

        assert!(html.contains("Directory listing for /sub/"));
        assert!(html.contains("../"));
        assert!(html.contains("data.bin"));
        assert_eq!(count, 1);
    }

    #[test]
    fn test_generate_dir_listing_empty_dir() {
        let dir = tempfile::TempDir::new().expect("failed to create temp dir");

        let (html, count) = generate_dir_listing(dir.path(), "/empty/");

        assert!(html.contains("Directory listing for /empty/"));
        assert!(html.contains("../"));
        assert_eq!(count, 0);
    }

    #[test]
    fn test_generate_dir_listing_dirs_before_files() {
        let dir = tempfile::TempDir::new().expect("failed to create temp dir");
        std::fs::create_dir(dir.path().join("zzz_dir")).unwrap();
        let mut f = std::fs::File::create(dir.path().join("aaa_file.txt")).unwrap();
        f.write_all(b"x").unwrap();

        let (html, count) = generate_dir_listing(dir.path(), "/");

        assert_eq!(count, 2);

        let zzz_pos = html.find("zzz_dir").unwrap();
        let aaa_pos = html.find("aaa_file").unwrap();
        assert!(zzz_pos < aaa_pos, "directories should appear before files");
    }
}
