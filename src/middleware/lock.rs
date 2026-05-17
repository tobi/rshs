use std::sync::Arc;

use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::server::AppState;
use crate::webdav::{self, IfCondition, LockInfo};

fn eval_condition(cond: &IfCondition, infos: &[LockInfo]) -> bool {
    match cond {
        IfCondition::StateToken(t) if t == "DAV:no-lock" => !infos.iter().any(|l| l.is_exclusive()),
        IfCondition::StateToken(t) => infos.iter().any(|l| l.token == *t),
        IfCondition::Not(inner) => !eval_condition(inner, infos),
    }
}

fn evaluate_if(lists: &[webdav::IfList], infos: &[LockInfo], request_path: &str) -> bool {
    // No If header → the resource must be unlocked
    if lists.is_empty() {
        return infos.is_empty();
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

    // Check source path
    if let Ok(src) = state.resolve_and_guard(&request_path).await {
        let infos = match locks.get(&src) {
            Some(v) => v.as_slice(),
            None => &[],
        };
        if !evaluate_if(&lists, infos, &request_path) {
            tracing::debug!(path = %src.display(), "resource locked, rejecting write");
            return Err(StatusCode::LOCKED.into_response());
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
            }
        }
    }

    drop(locks);
    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

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
}
