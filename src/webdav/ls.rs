//! Lock system evaluation — ancestor walk, `If` condition evaluation, active-lock filtering.

use std::path::Path;

use axum::http::StatusCode;

use super::{Depth, IfCondition, IfList, LockInfo, LockStore};

/// Filter a lock slice to only active (non-expired) locks.
///
/// This is used internally by the lock evaluation engine to lazily skip
/// expired locks without modifying the store. Expired locks are pruned
/// separately by the periodic cleanup task in `start_server`.
pub fn active_slice(infos: &[LockInfo]) -> impl Iterator<Item = &LockInfo> + '_ {
    infos.iter().filter(|l| !l.is_expired())
}

/// Walk up the ancestor chain of `target` within `root_canonical`, calling
/// `f` with the lock slice at each ancestor directory.
///
/// When there are few depth:infinity locks (≤ 10), iterates locks directly
/// and checks ancestry via prefix comparison. Falls back to per-ancestor
/// HashMap walk when many depth:infinity locks exist.
///
/// Stops at the first ancestor where `f` returns `true`, or when the walk
/// reaches the root boundary.
///
/// Re-exported from [`crate::webdav::ls`].
pub fn walk_locked_ancestors<'a>(
    locks: &'a LockStore,
    target: &Path,
    root_canonical: &Path,
    mut f: impl FnMut(&'a [LockInfo]) -> bool,
) -> bool {
    if locks.is_empty() {
        return false;
    }

    let inf_entries: Vec<_> = locks
        .iter()
        .filter(|(_, infos)| infos.iter().any(|l| l.depth == Depth::Infinity))
        .collect();

    if inf_entries.is_empty() {
        return false;
    }

    if inf_entries.len() <= 10 {
        for (lock_path, infos) in inf_entries {
            if is_ancestor_of(lock_path, target) && f(infos) {
                return true;
            }
        }
        false
    } else {
        let mut current = target.parent();
        while let Some(parent) = current {
            if !parent.starts_with(root_canonical) {
                break;
            }
            if let Some(infos) = locks.get(parent) {
                if f(infos) {
                    return true;
                }
            }
            current = parent.parent();
        }
        false
    }
}

/// Returns `true` if `ancestor` is a proper ancestor directory of `target`.
///
/// A path `/a/b` is an ancestor of `/a/b/c/file.txt` — the byte immediately
/// after the ancestor prefix is `/`. Root `/` is always an ancestor.
fn is_ancestor_of(ancestor: &Path, target: &Path) -> bool {
    let an = ancestor.as_os_str().as_encoded_bytes();
    let ta = target.as_os_str().as_encoded_bytes();
    if an.is_empty() || an.len() >= ta.len() {
        return false;
    }
    if !target.starts_with(ancestor) {
        return false;
    }
    if an == b"/" {
        return true;
    }
    ta[an.len()] == b'/'
}

/// Find the first ancestor lock matching a predicate, walking from
/// `target` up to the root.
///
/// Only considers locks with `Depth::Infinity`. Used by the lock middleware
/// and the LOCK handler for lock discovery and refresh.
///
/// Re-exported from [`crate::webdav::ls`].
pub fn find_ancestor_lock<'a>(
    locks: &'a LockStore,
    target: &Path,
    root_canonical: &Path,
    predicate: impl Fn(&LockInfo) -> bool,
) -> Option<&'a LockInfo> {
    let mut result: Option<&'a LockInfo> = None;
    walk_locked_ancestors(locks, target, root_canonical, |infos| {
        for lock in infos {
            if lock.depth == Depth::Infinity && predicate(lock) {
                result = Some(lock);
                return true;
            }
        }
        false
    });
    result
}

/// Evaluate a single `If` header condition against an active lock slice.
///
/// - `StateToken("DAV:no-lock")`: true if no active locks exist.
/// - `StateToken(t)`: true if any active lock has token `t`.
/// - `Not(inner)`: negates the inner condition.
pub fn eval_condition(cond: &IfCondition, infos: &[LockInfo]) -> bool {
    match cond {
        IfCondition::StateToken(t) if t == "DAV:no-lock" => !active_slice(infos).any(|_| true),
        IfCondition::StateToken(t) => active_slice(infos).any(|l| l.token == *t),
        IfCondition::Not(inner) => !eval_condition(inner, infos),
    }
}

/// Evaluate a full `If` header (list of `IfList`s) against an active lock
/// slice for a given request path.
///
/// - Empty list: passes only if no active locks exist.
/// - Resource-tagged lists: skipped if the tag doesn't match `request_path`.
/// - All applicable lists must pass (AND); within a list, all conditions
///   must pass (AND); across lists the result is AND semantics.
pub fn eval_if(lists: &[IfList], infos: &[LockInfo], request_path: &str) -> bool {
    if lists.is_empty() {
        return active_slice(infos).next().is_none();
    }

    let mut applicable = lists
        .iter()
        .filter(|l| match &l.resource_tag {
            Some(tag) => tag == request_path,
            None => true,
        })
        .peekable();

    if applicable.peek().is_none() {
        return true;
    }

    applicable.all(|l| l.conditions.iter().all(|c| eval_condition(c, infos)))
}

/// Check whether an existing exclusive lock blocks a request.
///
/// Returns:
/// - `Ok(None)` if no exclusive lock exists (or it's expired / shared-only).
/// - `Ok(Some(token))` if `if_tokens` contains the matching lock token.
/// - `Err(LOCKED)` if an exclusive lock exists and `if_tokens` doesn't match.
///
/// Used by the lock enforcement middleware.
pub fn check_existing_exclusive(
    entry: &[LockInfo],
    if_tokens: &[String],
) -> Result<Option<String>, StatusCode> {
    let token_info = active_slice(entry).find(|l| l.is_exclusive());
    match token_info.map(|l| l.token.clone()) {
        Some(t) if if_tokens.contains(&t) => Ok(Some(t)),
        Some(_) => Err(StatusCode::LOCKED),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::webdav::LockScope;

    fn make_lock(scope: LockScope, token: &str) -> LockInfo {
        LockInfo::new(
            scope,
            token.into(),
            None,
            SystemTime::now(),
            None,
            Depth::Zero,
        )
    }

    fn make_expired_lock(token: &str) -> LockInfo {
        LockInfo::new(
            LockScope::Exclusive,
            token.into(),
            None,
            SystemTime::now() - Duration::from_secs(2),
            Some(Duration::from_secs(1)),
            Depth::Zero,
        )
    }

    #[test]
    fn test_eval_condition_state_token_match() {
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        let cond = IfCondition::StateToken("t1".into());
        assert!(eval_condition(&cond, &infos));
    }

    #[test]
    fn test_eval_condition_state_token_no_match() {
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        let cond = IfCondition::StateToken("t2".into());
        assert!(!eval_condition(&cond, &infos));
    }

    #[test]
    fn test_eval_condition_dav_no_lock_unlocked() {
        let infos: Vec<LockInfo> = vec![];
        let cond = IfCondition::StateToken("DAV:no-lock".into());
        assert!(eval_condition(&cond, &infos));
    }

    #[test]
    fn test_eval_condition_dav_no_lock_locked() {
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        let cond = IfCondition::StateToken("DAV:no-lock".into());
        assert!(!eval_condition(&cond, &infos));
    }

    #[test]
    fn test_eval_condition_not_token() {
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        let cond = IfCondition::Not(Box::new(IfCondition::StateToken("t2".into())));
        assert!(eval_condition(&cond, &infos));
    }

    #[test]
    fn test_eval_condition_not_token_reject() {
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        let cond = IfCondition::Not(Box::new(IfCondition::StateToken("t1".into())));
        assert!(!eval_condition(&cond, &infos));
    }

    #[test]
    fn test_eval_if_no_lists_unlocked() {
        let lists: Vec<IfList> = vec![];
        let infos: Vec<LockInfo> = vec![];
        assert!(eval_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_eval_if_no_lists_locked() {
        let lists: Vec<IfList> = vec![];
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        assert!(!eval_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_eval_if_matched_token() {
        let lists = vec![IfList::new(
            None,
            vec![IfCondition::StateToken("t1".into())],
        )];
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        assert!(eval_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_eval_if_wrong_token() {
        let lists = vec![IfList::new(
            None,
            vec![IfCondition::StateToken("t2".into())],
        )];
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        assert!(!eval_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_eval_if_not_no_lock_locked() {
        let lists = vec![IfList::new(
            None,
            vec![IfCondition::Not(Box::new(IfCondition::StateToken(
                "DAV:no-lock".into(),
            )))],
        )];
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        assert!(eval_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_eval_if_not_no_lock_unlocked() {
        let lists = vec![IfList::new(
            None,
            vec![IfCondition::Not(Box::new(IfCondition::StateToken(
                "DAV:no-lock".into(),
            )))],
        )];
        let infos: Vec<LockInfo> = vec![];
        assert!(!eval_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_eval_if_resource_tag_mismatch() {
        let lists = vec![IfList::new(
            Some("/b".into()),
            vec![IfCondition::StateToken("t2".into())],
        )];
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        assert!(eval_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_eval_if_resource_tag_match() {
        let lists = vec![IfList::new(
            Some("/a".into()),
            vec![IfCondition::StateToken("t1".into())],
        )];
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        assert!(eval_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_eval_if_no_lists_expired_ignored() {
        let lists: Vec<IfList> = vec![];
        let infos = vec![make_expired_lock("t1")];
        assert!(eval_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_dav_no_lock_expired_ignored() {
        let infos = vec![make_expired_lock("t1")];
        let cond = IfCondition::StateToken("DAV:no-lock".into());
        assert!(eval_condition(&cond, &infos));
    }

    #[test]
    fn test_token_match_expired_rejected() {
        let infos = vec![make_expired_lock("t1")];
        let cond = IfCondition::StateToken("t1".into());
        assert!(!eval_condition(&cond, &infos));
    }

    #[test]
    fn test_eval_if_dav_no_lock_unlocked() {
        let lists = vec![IfList::new(
            None,
            vec![IfCondition::StateToken("DAV:no-lock".into())],
        )];
        let infos: Vec<LockInfo> = vec![];
        assert!(eval_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_check_existing_exclusive_empty() {
        let entry: Vec<LockInfo> = vec![];
        let tokens: Vec<String> = vec![];
        assert_eq!(check_existing_exclusive(&entry, &tokens), Ok(None));
    }

    #[test]
    fn test_check_existing_exclusive_matching_token() {
        let entry = vec![make_lock(LockScope::Exclusive, "t1")];
        let tokens = vec!["t1".into()];
        assert_eq!(
            check_existing_exclusive(&entry, &tokens),
            Ok(Some("t1".into()))
        );
    }

    #[test]
    fn test_check_existing_exclusive_wrong_token() {
        let entry = vec![make_lock(LockScope::Exclusive, "t1")];
        let tokens = vec!["t2".into()];
        assert_eq!(
            check_existing_exclusive(&entry, &tokens),
            Err(StatusCode::LOCKED)
        );
    }

    #[test]
    fn test_check_existing_exclusive_expired_ignored() {
        let entry = vec![make_expired_lock("t1")];
        let tokens: Vec<String> = vec![];
        assert_eq!(check_existing_exclusive(&entry, &tokens), Ok(None));
    }

    #[test]
    fn test_check_existing_exclusive_shared_only() {
        let entry = vec![make_lock(LockScope::Shared, "t1")];
        let tokens: Vec<String> = vec![];
        assert_eq!(check_existing_exclusive(&entry, &tokens), Ok(None));
    }

    #[test]
    fn test_eval_if_dav_no_lock_locked() {
        let lists = vec![IfList::new(
            None,
            vec![IfCondition::StateToken("DAV:no-lock".into())],
        )];
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        assert!(!eval_if(&lists, &infos, "/a"));
    }

    // -- walk_locked_ancestors tests ------------------------------------------

    use std::collections::HashMap;
    use std::path::PathBuf;

    fn lock_store(entries: Vec<(&str, LockInfo)>) -> LockStore {
        let mut store = LockStore::new();
        for (path, info) in entries {
            store.entry(PathBuf::from(path)).or_default().push(info);
        }
        store
    }

    fn make_infinity_lock(scope: LockScope, token: &str) -> LockInfo {
        LockInfo::new(
            scope,
            token.into(),
            None,
            SystemTime::now(),
            None,
            Depth::Infinity,
        )
    }

    #[test]
    fn test_walk_locked_ancestors_empty_store() {
        let store: LockStore = HashMap::new();
        let target = Path::new("/a/b/c/file.txt");
        let root = Path::new("/a");
        assert!(!walk_locked_ancestors(&store, target, root, |_| true));
    }

    #[test]
    fn test_walk_locked_ancestors_no_infinity() {
        let store = lock_store(vec![("/a/b", make_lock(LockScope::Exclusive, "t1"))]);
        let target = Path::new("/a/b/c/file.txt");
        let root = Path::new("/a");
        assert!(!walk_locked_ancestors(&store, target, root, |_| true));
    }

    #[test]
    fn test_walk_locked_ancestors_ancestor_match() {
        let store = lock_store(vec![(
            "/a/b",
            make_infinity_lock(LockScope::Exclusive, "t1"),
        )]);
        let target = Path::new("/a/b/c/file.txt");
        let root = Path::new("/a");
        assert!(walk_locked_ancestors(&store, target, root, |_| true));
    }

    #[test]
    fn test_walk_locked_ancestors_no_match() {
        let store = lock_store(vec![(
            "/x/y",
            make_infinity_lock(LockScope::Exclusive, "t1"),
        )]);
        let target = Path::new("/a/b/c/file.txt");
        let root = Path::new("/a");
        assert!(!walk_locked_ancestors(&store, target, root, |_| true));
    }

    #[test]
    fn test_walk_locked_ancestors_path_boundary() {
        let store = lock_store(vec![(
            "/a/b",
            make_infinity_lock(LockScope::Exclusive, "t1"),
        )]);
        // /a/bc/file.txt starts_with /a/b but /a/b is NOT an ancestor
        let target = Path::new("/a/bc/file.txt");
        let root = Path::new("/a");
        assert!(!walk_locked_ancestors(&store, target, root, |_| true));
    }

    #[test]
    fn test_walk_locked_ancestors_root_lock() {
        let store = lock_store(vec![("/", make_infinity_lock(LockScope::Exclusive, "t1"))]);
        let target = Path::new("/anything/deep/file.txt");
        let root = Path::new("/");
        assert!(walk_locked_ancestors(&store, target, root, |_| true));
    }

    #[test]
    fn test_walk_locked_ancestors_many_locks() {
        let mut store = LockStore::new();
        for i in 0..15 {
            store
                .entry(PathBuf::from(format!("/level_{i}")))
                .or_default()
                .push(make_infinity_lock(LockScope::Exclusive, &format!("t{i}")));
        }
        // One of the locks matches an ancestor: /level_0
        let target = Path::new("/level_0/sub/file.txt");
        let root = Path::new("/");
        assert!(walk_locked_ancestors(&store, target, root, |_| true));
    }

    #[test]
    fn test_is_ancestor_of_boundary_cases() {
        // Normal
        assert!(is_ancestor_of(
            Path::new("/a/b"),
            Path::new("/a/b/c/file.txt")
        ));
        // Boundary: prefix trap
        assert!(!is_ancestor_of(
            Path::new("/a/b"),
            Path::new("/a/bc/file.txt")
        ));
        // Root
        assert!(is_ancestor_of(Path::new("/"), Path::new("/a")));
        assert!(is_ancestor_of(Path::new("/"), Path::new("/file.txt")));
        // Same path is not an ancestor
        assert!(!is_ancestor_of(Path::new("/a/b"), Path::new("/a/b")));
        // Ancestor longer than target
        assert!(!is_ancestor_of(Path::new("/a/b/c"), Path::new("/a/b")));
        // Empty ancestor
        assert!(!is_ancestor_of(Path::new(""), Path::new("/a")));
    }
}
