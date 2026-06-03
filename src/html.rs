//! HTML directory listing generation.
//!
//! Provides the HTML rendering and formatting helpers used by
//! [`handle_get_head`](crate::handlers::http::handle_get_head) to produce
//! directory index pages.  Directory enumeration is delegated to
//! [`scandir::batch_read_dir_entries`].

use std::fmt::Write;
use std::path::Path;

use percent_encoding::percent_decode_str;

use crate::scandir::{self, DirEntryMeta};
use crate::utils::time::format_rfc850;

/// Generate an HTML directory listing for a filesystem path.
///
/// Reads directory entries, sorts them (directories before files, then
/// alphabetically), and renders an HTML page with navigable links.
/// A [`../`] parent link is included for non-root paths.
///
/// Returns the HTML string and the number of directory entries. If the
/// directory cannot be read, returns a styled error page with entry count 0.
///
/// # Panics
///
/// Panics if writing to the in-memory HTML buffer fails.
/// In practice this never occurs — `String`'s `fmt::Write` implementation
/// is infallible except for out-of-memory, which Rust does not recover from.
///
/// This is the primary entry point used by
/// [`handle_get_head`](crate::handlers::http::handle_get_head) to serve
/// directory index pages to browsers.
pub(crate) async fn generate_dir_listing(dir_path: &Path, request_path: &str) -> (String, usize) {
    let mut html = String::new();

    let mut entries = match scandir::batch_read_dir_entries(dir_path).await {
        Ok(entries) => entries,
        Err(_) => {
            return (
                "<!DOCTYPE html><html><head><title>Error</title></head><body><h1>Cannot read directory</h1></body></html>"
                    .to_string(),
                0,
            );
        }
    };

    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));

    let (max_name_len, max_size_len) = entries.iter().fold((0, 0), |(mn, ms), e| {
        (mn.max(display_name_len(e)), ms.max(size_label_len(e)))
    });
    let name_col = max_name_len + 20;

    let decoded_path = percent_decode_str(request_path).decode_utf8_lossy();

    write!(
        html,
        "<!DOCTYPE html><html><head><title>Index of {decoded_path}</title><meta charset=\"utf-8\"></head><body><h1>Index of {decoded_path}</h1><hr><pre>"
    )
    .unwrap();

    if request_path != "/" {
        html.push_str("<a href=\"../\">../</a>\n");
    }

    for entry in &entries {
        let name = entry.name.to_string_lossy();

        if entry.is_dir {
            write!(html, "<a href=\"{name}/\">{name}/</a>").unwrap();
        } else {
            write!(html, "<a href=\"{name}\">{name}</a>").unwrap();
        }

        let pad1 = name_col.saturating_sub(display_name_len(entry));
        let size_str = size_label(entry);
        let date_str = format_rfc850(entry.modified);

        writeln!(
            html,
            "{:pad1$}{date_str}    {:>max_size_len$}",
            "", size_str
        )
        .unwrap();
    }

    html.push_str("</pre><hr></body></html>");

    (html, entries.len())
}

fn display_name_len(e: &DirEntryMeta) -> usize {
    e.name.len() + if e.is_dir { 1 } else { 0 }
}

fn size_label(e: &DirEntryMeta) -> String {
    if e.is_dir {
        "-".to_string()
    } else {
        e.size.to_string()
    }
}

fn size_label_len(e: &DirEntryMeta) -> usize {
    if e.is_dir {
        1
    } else {
        e.size.checked_ilog10().unwrap_or(0) as usize + 1
    }
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
