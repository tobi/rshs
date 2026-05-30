//! Filesystem traversal for PROPFIND — directory entry collection with href encoding.

use std::path::{Path, PathBuf};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use super::{Depth, PropEntry};
use crate::utils::scandir;

/// Characters that do NOT need percent-encoding in a WebDAV href path segment.
const HREF_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'/')
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Collect `PropEntry` entries from a filesystem path for PROPFIND responses.
///
/// Reads the metadata for `fs_path`, creates a base `PropEntry`, and — if the
/// target is a directory and `depth` allows — recursively collects child entries.
///
/// - `Depth::Zero`: only the target itself
/// - `Depth::One`: target + immediate children
/// - `Depth::Infinity`: target + full descendant tree
pub async fn collect_entries(fs_path: &Path, request_path: &str, depth: Depth) -> Vec<PropEntry> {
    let meta = match tokio::fs::metadata(fs_path).await {
        Ok(m) => m,
        Err(_) => return vec![],
    };

    let is_dir = meta.is_dir();
    let mut base_entry = PropEntry::from_meta(&meta, normalize_href(request_path, is_dir), is_dir);
    base_entry.canonical_path = Some(fs_path.to_path_buf());
    if !is_dir {
        base_entry.content_type = guess_content_type(&fs_path);
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

fn guess_content_type(child_name: &impl AsRef<Path>) -> Option<String> {
    mime_guess::from_path(child_name)
        .first()
        .map(|m| m.essence_str().to_owned())
}

async fn collect_direct_children(dir_path: &Path, parent_href: &str, entries: &mut Vec<PropEntry>) {
    let children = match scandir::batch_read_dir_entries(dir_path).await {
        Ok(c) => c,
        Err(_) => return,
    };

    let base = normalize_href(parent_href, true);

    for child in children {
        let name_str = child.name.to_string_lossy();
        let child_href = format!(
            "{base}{encoded}",
            encoded = utf8_percent_encode(&name_str, HREF_ENCODE_SET)
        );

        let mut entry = PropEntry::from_scandir(&child, child_href);
        if !child.is_dir {
            entry.content_type = guess_content_type(&child.name);
        }
        entries.push(entry);
    }
}

async fn collect_descendants(root_dir: &Path, root_href: &str, entries: &mut Vec<PropEntry>) {
    let mut stack: Vec<(PathBuf, String)> = Vec::new();
    stack.push((root_dir.to_path_buf(), normalize_href(root_href, true)));

    while let Some((dir_path, parent_href)) = stack.pop() {
        let children = match scandir::batch_read_dir_entries(&dir_path).await {
            Ok(c) => c,
            Err(_) => continue,
        };

        for child in children {
            let name_str = child.name.to_string_lossy();
            let child_href = format!(
                "{base}{encoded}",
                base = parent_href,
                encoded = utf8_percent_encode(&name_str, HREF_ENCODE_SET)
            );

            let mut entry = PropEntry::from_scandir(&child, child_href.clone());
            if !child.is_dir {
                entry.content_type = guess_content_type(&child.name);
            }
            entries.push(entry);

            if child.is_dir {
                let sub_href = normalize_href_dir(&child_href);
                stack.push((dir_path.join(&child.name), sub_href));
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
