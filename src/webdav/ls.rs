use std::path::Path;

use axum::http::StatusCode;

use super::{Depth, IfCondition, IfList, LockInfo, LockStore};

pub fn active_slice(infos: &[LockInfo]) -> impl Iterator<Item = &LockInfo> + '_ {
    infos.iter().filter(|l| !l.is_expired())
}

pub fn walk_locked_ancestors<'a>(
    locks: &'a LockStore,
    target: &Path,
    root_canonical: &Path,
    mut f: impl FnMut(&'a [LockInfo]) -> bool,
) -> bool {
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

pub fn eval_condition(cond: &IfCondition, infos: &[LockInfo]) -> bool {
    match cond {
        IfCondition::StateToken(t) if t == "DAV:no-lock" => !active_slice(infos).any(|_| true),
        IfCondition::StateToken(t) => active_slice(infos).any(|l| l.token == *t),
        IfCondition::Not(inner) => !eval_condition(inner, infos),
    }
}

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
        // Tag doesn't match /a → list is skipped → passes
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
        // Expired lock ignored lazily → effectively unlocked → passes
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
        // Expired lock ignored lazily → token no longer valid
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
}
