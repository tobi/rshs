//! HTML directory listing generation.
//!
//! Provides the data model (`DirEntry`), filesystem traversal, and HTML
//! rendering used by [`handle_get_head`](crate::handlers::http::handle_get_head)
//! to produce directory index pages.

use std::path::Path;
use std::time::SystemTime;

use derive_new::new;

use crate::utils::scandir;
use crate::utils::time::format_rfc850;

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

/// Generate an HTML directory listing for a filesystem path.
///
/// Reads directory entries, sorts them (directories before files, then
/// alphabetically), and renders an HTML page with navigable links.
/// A [`../`] parent link is included for non-root paths.
///
/// Returns the HTML string and the number of directory entries. If the
/// directory cannot be read, returns a styled error page with entry count 0.
///
/// This is the primary entry point used by
/// [`handle_get_head`](crate::handlers::http::handle_get_head) to serve
/// directory index pages to browsers.
pub(crate) async fn generate_dir_listing(dir_path: &Path, request_path: &str) -> (String, usize) {
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

async fn collect_dir_entries(dir_path: &Path) -> Option<Vec<DirEntry>> {
    let children = scandir::batch_read_dir_entries(dir_path).await.ok()?;

    let entries = children
        .into_iter()
        .map(|c| {
            DirEntry::new(
                c.name.to_string_lossy().into_owned(),
                c.is_dir,
                c.size,
                c.modified,
            )
        })
        .collect();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_generate_dir_listing_structure() {
        let dir = tempfile::TempDir::new().unwrap();
        use std::io::Write;
        let mut f = std::fs::File::create(dir.path().join("hello.txt")).unwrap();
        f.write_all(b"hello").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let (html, count) = generate_dir_listing(dir.path(), "/").await;

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

        let (html, count) = generate_dir_listing(dir.path(), "/sub/").await;

        assert!(html.contains("Index of /sub/"));
        assert!(html.contains("../"));
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_generate_dir_listing_empty_dir() {
        let dir = tempfile::TempDir::new().unwrap();

        let (html, count) = generate_dir_listing(dir.path(), "/empty/").await;

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

        let (html, count) = generate_dir_listing(dir.path(), "/").await;

        assert_eq!(count, 2);
        let zzz_pos = html.find("zzz_dir").unwrap();
        let aaa_pos = html.find("aaa_file").unwrap();
        assert!(zzz_pos < aaa_pos, "directories should appear before files");
    }
}
