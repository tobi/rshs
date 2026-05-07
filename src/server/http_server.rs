use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use actix_web::{HttpRequest, HttpResponse, http::header, web};

use super::time_util::format_modified;

pub async fn handle(req: HttpRequest, root_dir: web::Data<PathBuf>) -> HttpResponse {
    let request_path = req.path();

    let rel_path = request_path.trim_start_matches('/');
    let fs_path = root_dir.join(rel_path);

    tracing::debug!(method = %req.method(), path = request_path, "incoming request");

    let fs_path = match fs_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            tracing::debug!(method = %req.method(), path = request_path, status = 404, "path not found");
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
            "path traversal blocked",
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
                            "failed to read file",
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
                "method not allowed",
            );
            HttpResponse::MethodNotAllowed().finish()
        }
    }
}

fn generate_dir_listing(dir_path: &std::path::Path, request_path: &str) -> (String, usize) {
    let dir_entries: Vec<_> = match fs::read_dir(dir_path) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => {
            return ("<!DOCTYPE html>\n<html>\n<head>\n<title>Error</title>\n</head>\n<body>\n<h1>Cannot read directory</h1>\n</body>\n</html>\n".to_string(), 0);
        }
    };

    let mut entries: Vec<(String, bool, u64, SystemTime)> = dir_entries
        .into_iter()
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            (name, is_dir, size, modified)
        })
        .collect();

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

    #[test]
    fn test_generate_dir_listing_structure() {
        let dir = tempfile::TempDir::new().expect("failed to create temp dir");
        let mut f = std::fs::File::create(dir.path().join("hello.txt")).unwrap();
        f.write_all(b"hello").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let (html, count) = generate_dir_listing(dir.path(), "/");

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<title>Index of /</title>"));
        assert!(html.contains("<pre>"));
        assert!(html.contains("hello.txt"));
        assert!(html.contains("subdir/"));
        assert!(!html.contains("../"));
        assert!(!html.contains("<ul>"));
        assert!(!html.contains("<li>"));
        assert_eq!(count, 2);
    }

    #[test]
    fn test_generate_dir_listing_subdir_has_parent_link() {
        let dir = tempfile::TempDir::new().expect("failed to create temp dir");
        let mut f = std::fs::File::create(dir.path().join("data.bin")).unwrap();
        f.write_all(b"bin").unwrap();

        let (html, count) = generate_dir_listing(dir.path(), "/sub/");

        assert!(html.contains("Index of /sub/"));
        assert!(html.contains("../"));
        assert!(html.contains("data.bin"));
        assert_eq!(count, 1);
    }

    #[test]
    fn test_generate_dir_listing_empty_dir() {
        let dir = tempfile::TempDir::new().expect("failed to create temp dir");

        let (html, count) = generate_dir_listing(dir.path(), "/empty/");

        assert!(html.contains("Index of /empty/"));
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
