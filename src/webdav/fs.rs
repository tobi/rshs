use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use super::{Depth, PropEntry};

/// Characters that do NOT need percent-encoding in a WebDAV href path segment.
pub const HREF_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'/')
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

pub async fn collect_entries(fs_path: &Path, request_path: &str, depth: Depth) -> Vec<PropEntry> {
    let meta = match tokio::fs::metadata(fs_path).await {
        Ok(m) => m,
        Err(_) => return vec![],
    };

    let is_dir = meta.is_dir();
    let base_entry = PropEntry {
        href: normalize_href(request_path, is_dir),
        is_dir,
        size: meta.len(),
        modified: meta.modified().unwrap_or(UNIX_EPOCH),
    };

    let mut entries = vec![base_entry];

    if is_dir && depth != Depth::Zero {
        if depth == Depth::One {
            collect_direct_children(fs_path, request_path, &mut entries).await;
        } else {
            collect_descendants(fs_path, request_path, &mut entries).await;
        }
    }

    entries
}

async fn collect_direct_children(dir_path: &Path, parent_href: &str, entries: &mut Vec<PropEntry>) {
    let mut read_dir = match tokio::fs::read_dir(dir_path).await {
        Ok(rd) => rd,
        Err(_) => return,
    };

    let base = normalize_href(parent_href, true);

    loop {
        let dir_entry = match read_dir.next_entry().await {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(_) => continue,
        };

        let name = dir_entry.file_name();
        let name_str = name.to_string_lossy();
        let meta = match dir_entry.metadata().await {
            Ok(m) => m,
            Err(_) => continue,
        };

        let file_type = meta.file_type();
        let child_href = format!(
            "{base}{encoded}",
            encoded = utf8_percent_encode(&name_str, HREF_ENCODE_SET)
        );

        entries.push(PropEntry {
            href: child_href,
            is_dir: file_type.is_dir(),
            size: meta.len(),
            modified: meta.modified().unwrap_or(UNIX_EPOCH),
        });
    }
}

async fn collect_descendants(root_dir: &Path, root_href: &str, entries: &mut Vec<PropEntry>) {
    let mut stack: Vec<(PathBuf, String)> = Vec::new();
    stack.push((root_dir.to_path_buf(), normalize_href(root_href, true)));

    while let Some((dir_path, parent_href)) = stack.pop() {
        let mut read_dir = match tokio::fs::read_dir(&dir_path).await {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        loop {
            let dir_entry = match read_dir.next_entry().await {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(_) => continue,
            };

            let name = dir_entry.file_name();
            let name_str = name.to_string_lossy();
            let meta = match dir_entry.metadata().await {
                Ok(m) => m,
                Err(_) => continue,
            };

            let file_type = meta.file_type();
            let child_href = format!(
                "{base}{encoded}",
                base = parent_href,
                encoded = utf8_percent_encode(&name_str, HREF_ENCODE_SET)
            );

            entries.push(PropEntry {
                href: child_href.clone(),
                is_dir: file_type.is_dir(),
                size: meta.len(),
                modified: meta.modified().unwrap_or(UNIX_EPOCH),
            });

            if file_type.is_dir() {
                let sub_href = normalize_href_dir(&child_href);
                stack.push((dir_entry.path(), sub_href));
            }
        }
    }
}

fn normalize_href(path: &str, is_dir: bool) -> String {
    if is_dir && !path.ends_with('/') {
        format!("{path}/")
    } else {
        path.to_owned()
    }
}

fn normalize_href_dir(path: &str) -> String {
    if path.ends_with('/') {
        path.to_owned()
    } else {
        format!("{path}/")
    }
}
