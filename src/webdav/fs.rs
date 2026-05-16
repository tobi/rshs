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

fn make_entry(
    href: String,
    is_dir: bool,
    meta: &std::fs::Metadata,
    canonical: Option<PathBuf>,
) -> PropEntry {
    PropEntry {
        href,
        is_dir,
        size: meta.len(),
        modified: meta.modified().unwrap_or(UNIX_EPOCH),
        created: meta.created().ok(),
        content_type: None,
        dead_props: None,
        canonical_path: canonical,
    }
}

fn guess_content_type(child_name: &std::ffi::OsStr) -> Option<String> {
    let mime = mime_guess::from_path(child_name).first_or_octet_stream();
    if mime == mime_guess::mime::APPLICATION_OCTET_STREAM {
        None
    } else {
        Some(mime.essence_str().to_owned())
    }
}

pub async fn collect_entries(fs_path: &Path, request_path: &str, depth: Depth) -> Vec<PropEntry> {
    let meta = match tokio::fs::metadata(fs_path).await {
        Ok(m) => m,
        Err(_) => return vec![],
    };

    let is_dir = meta.is_dir();
    let mut base_entry = make_entry(
        normalize_href(request_path, is_dir),
        is_dir,
        &meta,
        Some(fs_path.to_path_buf()),
    );
    if !is_dir {
        base_entry.content_type = guess_content_type(fs_path.as_os_str());
    }

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

        let is_dir = meta.file_type().is_dir();
        let child_href = format!(
            "{base}{encoded}",
            encoded = utf8_percent_encode(&name_str, HREF_ENCODE_SET)
        );

        let mut entry = make_entry(child_href, is_dir, &meta, None);
        if !is_dir {
            entry.content_type = guess_content_type(&name);
        }
        entries.push(entry);
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

            let is_dir = meta.file_type().is_dir();
            let child_href = format!(
                "{base}{encoded}",
                base = parent_href,
                encoded = utf8_percent_encode(&name_str, HREF_ENCODE_SET)
            );

            let mut entry = make_entry(child_href.clone(), is_dir, &meta, None);
            if !is_dir {
                entry.content_type = guess_content_type(&name);
            }
            entries.push(entry);

            if is_dir {
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
