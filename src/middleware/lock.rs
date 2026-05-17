use std::path::Path;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::server::AppState;
use crate::webdav::{self, Depth, IfCondition, LockInfo, LockStore};

pub async fn lock_enforce(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    req: axum::extract::Request,
    next: Next,
) -> Result<Response, Response> {
    let method = req.method().as_str();

    if !matches!(
        method,
        "PUT" | "DELETE" | "MKCOL" | "PROPPATCH" | "MOVE" | "COPY"
    ) {
        return Ok(next.run(req).await);
    }

    let request_path = req.uri().path().trim_end_matches('/').to_owned();
    let lists = webdav::parse_if_header(req.headers());

    let locks = state.locks.read().await;

    // Check source path (skip for COPY — source is read-only)
    if method != "COPY" {
        if let Ok(src) = state.resolve_and_guard(&request_path).await {
            let infos = match locks.get(&src) {
                Some(v) => v.as_slice(),
                None => &[],
            };
            if !evaluate_if(&lists, infos, &request_path) {
                tracing::debug!(path = %src.display(), "resource locked, rejecting write");
                return Err(StatusCode::LOCKED.into_response());
            }
            // Check ancestor depth:infinity locks
            if let Some(status) = check_depth_infinity_ancestors(
                &locks,
                &src,
                &lists,
                &state.root_canonical,
                &request_path,
            ) {
                tracing::debug!(path = %request_path, "ancestor depth:infinity lock");
                return Err(status.into_response());
            }
        }
    }

    // For COPY/MOVE, additionally check destination path
    if method == "COPY" || method == "MOVE" {
        if let Some(dest) = webdav::parse_destination(req.headers()) {
            if let Ok(dest_path) = state.resolve_and_guard(dest.trim_end_matches('/')).await {
                let dest_normalized = dest.trim_end_matches('/');
                let infos = match locks.get(&dest_path) {
                    Some(v) => v.as_slice(),
                    None => &[],
                };
                if !evaluate_if(&lists, infos, dest_normalized) {
                    tracing::debug!(path = %dest_path.display(), "destination locked, rejecting COPY/MOVE");
                    return Err(StatusCode::LOCKED.into_response());
                }
                // Check ancestor depth:infinity locks on destination
                if let Some(status) = check_depth_infinity_ancestors(
                    &locks,
                    &dest_path,
                    &lists,
                    &state.root_canonical,
                    dest_normalized,
                ) {
                    tracing::debug!(path = %dest_normalized, "ancestor depth:infinity lock on destination");
                    return Err(status.into_response());
                }
            }
        }
    }

    drop(locks);
    Ok(next.run(req).await)
}

fn active_lock(infos: &[LockInfo]) -> impl Iterator<Item = &LockInfo> + '_ {
    infos.iter().filter(|l| !l.is_expired())
}

fn eval_condition(cond: &IfCondition, infos: &[LockInfo]) -> bool {
    match cond {
        #[cfg(not(feature = "litmus-compat"))]
        IfCondition::StateToken(t) if t == "DAV:no-lock" => !active_lock(infos).any(|_| true),
        #[cfg(feature = "litmus-compat")]
        IfCondition::StateToken(t) if t == "DAV:no-lock" => false,
        IfCondition::StateToken(t) => active_lock(infos).any(|l| l.token == *t),
        IfCondition::Not(inner) => !eval_condition(inner, infos),
    }
}

fn evaluate_if(lists: &[webdav::IfList], infos: &[LockInfo], request_path: &str) -> bool {
    // No If header → the resource must be unlocked
    if lists.is_empty() {
        return !active_lock(infos).any(|_| true);
    }

    // Filter lists applicable to this resource
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

    // All applicable lists must pass (AND semantics across lists)
    applicable
        .iter()
        .all(|l| l.conditions.iter().all(|c| eval_condition(c, infos)))
}

fn check_depth_infinity_ancestors(
    locks: &LockStore,
    target: &Path,
    lists: &[webdav::IfList],
    root_canonical: &Path,
    request_path: &str,
) -> Option<StatusCode> {
    let mut current = target.parent();
    while let Some(parent) = current {
        if !parent.starts_with(root_canonical) {
            break;
        }
        if let Some(infos) = locks.get(parent) {
            if active_lock(infos).any(|l| l.depth == Depth::Infinity)
                && !evaluate_if(lists, infos, request_path)
            {
                return Some(StatusCode::LOCKED);
            }
        }
        current = parent.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::webdav::{Depth, IfCondition, IfList, LockInfo, LockScope};

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

    #[cfg(not(feature = "litmus-compat"))]
    #[test]
    fn test_eval_condition_dav_no_lock_unlocked() {
        let infos: Vec<LockInfo> = vec![];
        let cond = IfCondition::StateToken("DAV:no-lock".into());
        assert!(eval_condition(&cond, &infos));
    }

    #[cfg(feature = "litmus-compat")]
    #[test]
    fn test_eval_condition_dav_no_lock_unlocked_always_fails() {
        let infos: Vec<LockInfo> = vec![];
        let cond = IfCondition::StateToken("DAV:no-lock".into());
        assert!(!eval_condition(&cond, &infos));
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

    #[cfg(not(feature = "litmus-compat"))]
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

    #[cfg(feature = "litmus-compat")]
    #[test]
    fn test_evaluate_if_not_no_lock_unlocked_always_passes() {
        let lists = vec![IfList {
            resource_tag: None,
            conditions: vec![IfCondition::Not(Box::new(IfCondition::StateToken(
                "DAV:no-lock".into(),
            )))],
        }];
        let infos: Vec<LockInfo> = vec![];
        assert!(evaluate_if(&lists, &infos, "/a"));
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

    #[cfg(not(feature = "litmus-compat"))]
    #[test]
    fn test_dav_no_lock_expired_ignored() {
        let infos = vec![make_expired_lock("t1")];
        let cond = IfCondition::StateToken("DAV:no-lock".into());
        assert!(eval_condition(&cond, &infos));
    }

    #[cfg(feature = "litmus-compat")]
    #[test]
    fn test_dav_no_lock_expired_still_fails() {
        let infos = vec![make_expired_lock("t1")];
        let cond = IfCondition::StateToken("DAV:no-lock".into());
        // Under litmus-compat, DAV:no-lock always fails
        assert!(!eval_condition(&cond, &infos));
    }

    #[test]
    fn test_token_match_expired_rejected() {
        let infos = vec![make_expired_lock("t1")];
        let cond = IfCondition::StateToken("t1".into());
        // Expired lock ignored lazily → token no longer valid
        assert!(!eval_condition(&cond, &infos));
    }
}
