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

pub fn evaluate_if(lists: &[IfList], infos: &[LockInfo], request_path: &str) -> bool {
    if lists.is_empty() {
        return !active_slice(infos).any(|_| true);
    }

    let applicable: Vec<_> = lists
        .iter()
        .filter(|l| match &l.resource_tag {
            Some(tag) => tag == request_path,
            None => true,
        })
        .collect();

    if applicable.is_empty() {
        return true;
    }

    applicable
        .iter()
        .all(|l| l.conditions.iter().all(|c| eval_condition(c, infos)))
}

pub fn check_existing_exclusive(
    entry: &[LockInfo],
    if_tokens: &[String],
) -> Result<Option<String>, StatusCode> {
    let token = entry
        .iter()
        .find(|l| l.is_exclusive())
        .map(|l| l.token.clone());
    match token {
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
        LockInfo {
            scope,
            token: token.into(),
            owner: None,
            timeout: None,
            created: SystemTime::now(),
            depth: Depth::Zero,
        }
    }

    fn make_expired_lock(token: &str) -> LockInfo {
        LockInfo {
            scope: LockScope::Exclusive,
            token: token.into(),
            owner: None,
            timeout: Some(Duration::from_secs(1)),
            created: SystemTime::now() - Duration::from_secs(2),
            depth: Depth::Zero,
        }
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
    fn test_evaluate_if_no_lists_unlocked() {
        let lists: Vec<IfList> = vec![];
        let infos: Vec<LockInfo> = vec![];
        assert!(evaluate_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_evaluate_if_no_lists_locked() {
        let lists: Vec<IfList> = vec![];
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        assert!(!evaluate_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_evaluate_if_matched_token() {
        let lists = vec![IfList {
            resource_tag: None,
            conditions: vec![IfCondition::StateToken("t1".into())],
        }];
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        assert!(evaluate_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_evaluate_if_wrong_token() {
        let lists = vec![IfList {
            resource_tag: None,
            conditions: vec![IfCondition::StateToken("t2".into())],
        }];
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        assert!(!evaluate_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_evaluate_if_not_no_lock_locked() {
        let lists = vec![IfList {
            resource_tag: None,
            conditions: vec![IfCondition::Not(Box::new(IfCondition::StateToken(
                "DAV:no-lock".into(),
            )))],
        }];
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        assert!(evaluate_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_evaluate_if_not_no_lock_unlocked() {
        let lists = vec![IfList {
            resource_tag: None,
            conditions: vec![IfCondition::Not(Box::new(IfCondition::StateToken(
                "DAV:no-lock".into(),
            )))],
        }];
        let infos: Vec<LockInfo> = vec![];
        assert!(!evaluate_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_evaluate_if_resource_tag_mismatch() {
        let lists = vec![IfList {
            resource_tag: Some("/b".into()),
            conditions: vec![IfCondition::StateToken("t2".into())],
        }];
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        // Tag doesn't match /a → list is skipped → passes
        assert!(evaluate_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_evaluate_if_resource_tag_match() {
        let lists = vec![IfList {
            resource_tag: Some("/a".into()),
            conditions: vec![IfCondition::StateToken("t1".into())],
        }];
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        assert!(evaluate_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_evaluate_if_no_lists_expired_ignored() {
        let lists: Vec<IfList> = vec![];
        let infos = vec![make_expired_lock("t1")];
        // Expired lock ignored lazily → effectively unlocked → passes
        assert!(evaluate_if(&lists, &infos, "/a"));
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
    fn test_evaluate_if_dav_no_lock_unlocked() {
        let lists = vec![IfList {
            resource_tag: None,
            conditions: vec![IfCondition::StateToken("DAV:no-lock".into())],
        }];
        let infos: Vec<LockInfo> = vec![];
        assert!(evaluate_if(&lists, &infos, "/a"));
    }

    #[test]
    fn test_evaluate_if_dav_no_lock_locked() {
        let lists = vec![IfList {
            resource_tag: None,
            conditions: vec![IfCondition::StateToken("DAV:no-lock".into())],
        }];
        let infos = vec![make_lock(LockScope::Exclusive, "t1")];
        assert!(!evaluate_if(&lists, &infos, "/a"));
    }
}
