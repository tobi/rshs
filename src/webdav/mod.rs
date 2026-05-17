pub mod fs;
pub mod xml;

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

use axum::http::{HeaderMap, Method as HttpMethod};
use percent_encoding::percent_decode_str;
use quick_xml::Reader;
use quick_xml::events::Event;

type Method = LazyLock<HttpMethod>;

pub static M_PROPFIND: Method = LazyLock::new(|| HttpMethod::from_bytes(b"PROPFIND").unwrap());
pub static M_MKCOL: Method = LazyLock::new(|| HttpMethod::from_bytes(b"MKCOL").unwrap());
pub static M_COPY: Method = LazyLock::new(|| HttpMethod::from_bytes(b"COPY").unwrap());
pub static M_MOVE: Method = LazyLock::new(|| HttpMethod::from_bytes(b"MOVE").unwrap());
pub static M_PROPPATCH: Method = LazyLock::new(|| HttpMethod::from_bytes(b"PROPPATCH").unwrap());
pub static M_LOCK: Method = LazyLock::new(|| HttpMethod::from_bytes(b"LOCK").unwrap());
pub static M_UNLOCK: Method = LazyLock::new(|| HttpMethod::from_bytes(b"UNLOCK").unwrap());

pub type DeadPropertyStore = HashMap<PathBuf, HashMap<String, String>>;

pub struct PropPatchOp {
    pub set: HashMap<String, String>,
    pub remove: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    Zero,
    One,
    Infinity,
}

#[derive(Debug, Clone)]
pub enum PropRequest {
    AllProp,
    PropName,
    Named(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct PropEntry {
    pub canonical_path: Option<PathBuf>,
    pub href: String,
    pub content_type: Option<String>,
    pub modified: SystemTime,
    pub created: Option<SystemTime>,
    pub size: u64,
    pub is_dir: bool,
    pub dead_props: Option<HashMap<String, String>>,
    pub active_locks: Option<Vec<LockInfo>>,
}

pub type LockStore = HashMap<PathBuf, Vec<LockInfo>>;

#[derive(Debug, Clone)]
pub struct LockInfo {
    pub scope: LockScope,
    pub token: String,
    pub owner: Option<String>,
    pub created: SystemTime,
    pub timeout: Option<Duration>,
    pub depth: Depth,
}

impl LockInfo {
    pub fn is_expired(&self) -> bool {
        let Some(timeout) = self.timeout else {
            return false;
        };
        self.created.elapsed().unwrap_or_default() >= timeout
    }

    pub fn is_exclusive(&self) -> bool {
        matches!(self.scope, LockScope::Exclusive)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum LockScope {
    Exclusive,
    Shared,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IfCondition {
    StateToken(String),
    Not(Box<IfCondition>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfList {
    pub resource_tag: Option<String>,
    pub conditions: Vec<IfCondition>,
}

impl IfList {
    pub fn positive_tokens(&self) -> Vec<&str> {
        self.conditions
            .iter()
            .filter_map(|c| match c {
                IfCondition::StateToken(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }
}

pub fn generate_lock_token() -> String {
    use std::hash::{Hash, Hasher};
    use std::time::UNIX_EPOCH;
    let nanos = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    nanos.hash(&mut h);
    format!("opaquelocktoken:{:016x}", h.finish())
}

pub fn find_ancestor_lock<'a, F>(
    locks: &'a LockStore,
    target: &std::path::Path,
    root_canonical: &std::path::Path,
    predicate: F,
) -> Option<&'a LockInfo>
where
    F: Fn(&LockInfo) -> bool,
{
    let mut current = target.parent();
    while let Some(parent) = current {
        if !parent.starts_with(root_canonical) {
            break;
        }
        if let Some(infos) = locks.get(parent) {
            if let Some(lock) = infos
                .iter()
                .find(|l| l.depth == Depth::Infinity && predicate(l))
            {
                return Some(lock);
            }
        }
        current = parent.parent();
    }
    None
}

pub fn parse_if_header(headers: &HeaderMap) -> Vec<IfList> {
    let value = match headers.get("if").and_then(|v| v.to_str().ok()) {
        Some(v) => v,
        None => return Vec::new(),
    };

    let mut lists = Vec::new();
    let mut chars: std::iter::Peekable<_> = value.char_indices().peekable();

    while chars.peek().is_some() {
        // Skip whitespace
        while chars.peek().is_some_and(|(_, c)| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }

        let mut resource_tag = None;

        // Check for resource tag: <url> followed by (
        if chars.peek().unwrap().1 == '<' {
            // Save working position in case this isn't a resource tag
            let mut saved: Vec<_> = Vec::new();
            loop {
                let c = chars.next().unwrap().1;
                saved.push(c);
                if c == '>' {
                    break;
                }
            }

            let tag: String = saved[1..saved.len() - 1].iter().collect();

            // Check if next non-whitespace is '('
            let mut peek_pos = chars.peek();
            while peek_pos.is_some_and(|(_, c)| c.is_whitespace()) {
                chars.next();
                peek_pos = chars.peek();
            }
            if peek_pos.is_some_and(|(_, c)| *c == '(') {
                resource_tag = Some(tag);
                chars.next(); // consume '('
            } else {
                // Bare token without enclosing (...) — single-condition list
                lists.push(IfList {
                    resource_tag: None,
                    conditions: vec![IfCondition::StateToken(tag)],
                });
                continue;
            }
        } else if chars.peek().unwrap().1 == '(' {
            chars.next(); // consume '('
        } else {
            // Skip unexpected char
            chars.next();
            continue;
        }

        // Parse conditions inside (...) until ')'
        let mut conditions = Vec::new();
        let mut negated = false;

        loop {
            while chars.peek().is_some_and(|(_, c)| c.is_whitespace()) {
                chars.next();
            }

            match chars.peek() {
                None => break,
                Some((_, ')')) => {
                    chars.next();
                    break;
                }
                Some((i, _)) => {
                    // Check for "Not" keyword
                    if value[*i..].starts_with("Not") {
                        let after_not = &value[*i + 3..];
                        if after_not
                            .starts_with(|c: char| c.is_whitespace() || c == '<' || c == '(')
                        {
                            negated = true;
                            for _ in 0..3 {
                                chars.next();
                            }
                            continue;
                        }
                    }

                    // Read <token>
                    if chars.peek().unwrap().1 == '<' {
                        let mut token = String::new();
                        chars.next(); // skip '<'
                        loop {
                            match chars.next() {
                                Some((_, '>')) => break,
                                Some((_, c)) => token.push(c),
                                None => break,
                            }
                        }

                        let cond = IfCondition::StateToken(token);
                        if negated {
                            conditions.push(IfCondition::Not(Box::new(cond)));
                            negated = false;
                        } else {
                            conditions.push(cond);
                        }
                    } else {
                        // Skip unexpected char inside list
                        chars.next();
                    }
                }
            }
        }

        lists.push(IfList {
            resource_tag,
            conditions,
        });
    }

    lists
}

pub fn parse_lock_token_header(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("lock-token")?.to_str().ok()?;
    value
        .trim_matches('<')
        .trim_matches('>')
        .trim()
        .to_string()
        .into()
}

pub fn parse_timeout(headers: &HeaderMap) -> Option<std::time::Duration> {
    let value = headers.get("timeout")?.to_str().ok()?;
    let seconds = value
        .strip_prefix("Second-")
        .and_then(|s| s.parse::<u64>().ok())?;
    Some(std::time::Duration::from_secs(seconds))
}

pub fn parse_depth(headers: &HeaderMap) -> Depth {
    let depth = headers.get("depth");
    match depth.and_then(|v| v.to_str().ok()).unwrap_or("infinity") {
        "0" => Depth::Zero,
        "1" => Depth::One,
        _ => Depth::Infinity,
    }
}

#[derive(Debug)]
pub enum ParseError {
    InvalidBody(&'static str),
    Xml(quick_xml::Error),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Xml(e) => write!(f, "XML parse error: {e}"),
            Self::InvalidBody(s) => write!(f, "invalid body: {s}"),
        }
    }
}

impl From<quick_xml::Error> for ParseError {
    fn from(e: quick_xml::Error) -> Self {
        Self::Xml(e)
    }
}

pub fn parse_propfind_request(xml: &[u8]) -> Result<PropRequest, ParseError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);

    let mut props = Vec::new();
    let mut in_prop = false;
    let mut found_allprop = false;
    let mut found_propname = false;

    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                let local = e.local_name();
                let name = local.as_ref();
                match name {
                    b"prop" => in_prop = true,
                    b"allprop" => found_allprop = true,
                    b"propname" => found_propname = true,
                    _ if in_prop => {
                        props.push(String::from_utf8_lossy(name).to_string());
                    }
                    _ => {}
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"prop" => {
                in_prop = false;
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if found_allprop || props.is_empty() {
        Ok(PropRequest::AllProp)
    } else if found_propname {
        Ok(PropRequest::PropName)
    } else {
        Ok(PropRequest::Named(props))
    }
}

/// Extract the path from a Destination header (full URL → decoded path).
/// Trailing slashes are stripped for COPY/MOVE compatibility with litmus.
pub fn parse_destination(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("destination")?.to_str().ok()?;
    let mut path = if let Some(pos) = value.find("://") {
        let after_scheme = &value[pos + 3..];
        if let Some(slash_pos) = after_scheme.find('/') {
            percent_decode_str(&after_scheme[slash_pos..])
                .decode_utf8_lossy()
                .to_string()
        } else {
            return None;
        }
    } else if value.starts_with('/') {
        percent_decode_str(value).decode_utf8_lossy().to_string()
    } else {
        return None;
    };
    path.truncate(path.trim_end_matches('/').len());
    Some(path)
}

pub fn parse_overwrite(headers: &HeaderMap) -> bool {
    headers
        .get("overwrite")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_ascii_uppercase())
        .unwrap_or_else(|| "T".into())
        != "F"
}

pub fn parse_proppatch_request(xml: &[u8]) -> Result<PropPatchOp, ParseError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);

    let mut set_props = HashMap::new();
    let mut remove_props = Vec::new();
    let mut in_set = false;
    let mut in_remove = false;
    let mut found_any = false;
    let mut current_name: Option<String> = None;

    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref();
                match name {
                    b"set" => {
                        in_set = true;
                        found_any = true;
                    }
                    b"remove" => {
                        in_remove = true;
                        found_any = true;
                    }
                    _ if in_set && name != b"prop" => {
                        current_name = Some(String::from_utf8_lossy(name).to_string());
                    }
                    _ if in_remove && name != b"prop" && name != b"set" => {
                        remove_props.push(String::from_utf8_lossy(name).to_string());
                    }
                    _ => {}
                }
            }
            Event::Empty(e) => {
                let local = e.local_name();
                let name = local.as_ref();
                if in_remove && name != b"prop" {
                    remove_props.push(String::from_utf8_lossy(name).to_string());
                } else if in_set && name != b"prop" {
                    set_props.insert(String::from_utf8_lossy(name).to_string(), String::new());
                }
            }
            Event::Text(t) if in_set && current_name.is_some() => {
                let val = String::from_utf8_lossy(t.as_ref()).to_string();
                set_props.insert(current_name.take().unwrap(), val);
            }
            Event::End(e) => {
                let local = e.local_name();
                let name = local.as_ref();
                match name {
                    b"set" => in_set = false,
                    b"remove" => in_remove = false,
                    _ if in_set && current_name.is_some() => {
                        set_props.insert(current_name.take().unwrap(), String::new());
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if !found_any {
        return Err(ParseError::InvalidBody("invalid PROPPATCH body"));
    }

    Ok(PropPatchOp {
        set: set_props,
        remove: remove_props,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_lock_info(timeout: Option<Duration>, created_offset: Duration) -> LockInfo {
        LockInfo {
            token: "opaquelocktoken:test".into(),
            scope: LockScope::Exclusive,
            owner: None,
            timeout,
            created: SystemTime::now() - created_offset,
            depth: Depth::Zero,
        }
    }

    #[test]
    fn test_is_expired_no_timeout() {
        let lock = make_lock_info(None, Duration::from_secs(3600));
        assert!(!lock.is_expired());
    }

    #[test]
    fn test_is_expired_future() {
        let lock = make_lock_info(Some(Duration::from_secs(100)), Duration::ZERO);
        assert!(!lock.is_expired());
    }

    #[test]
    fn test_is_expired_past() {
        let lock = make_lock_info(Some(Duration::from_secs(1)), Duration::from_secs(2));
        assert!(lock.is_expired());
    }

    #[test]
    fn test_is_expired_exact_boundary() {
        let lock = make_lock_info(Some(Duration::from_secs(5)), Duration::from_secs(5));
        assert!(lock.is_expired());
    }

    fn make_if_header(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("if", value.parse().unwrap());
        headers
    }

    #[test]
    fn test_parse_if_simple_token() {
        let headers = make_if_header("(<opaquelocktoken:t1>)");
        let lists = parse_if_header(&headers);
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].resource_tag, None);
        assert_eq!(lists[0].conditions.len(), 1);
        assert_eq!(
            lists[0].conditions[0],
            IfCondition::StateToken("opaquelocktoken:t1".into())
        );
    }

    #[test]
    fn test_parse_if_not_token() {
        let headers = make_if_header("(Not <opaquelocktoken:t1>)");
        let lists = parse_if_header(&headers);
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].conditions.len(), 1);
        assert_eq!(
            lists[0].conditions[0],
            IfCondition::Not(Box::new(IfCondition::StateToken(
                "opaquelocktoken:t1".into()
            )))
        );
    }

    #[test]
    fn test_parse_if_not_no_lock() {
        let headers = make_if_header("(Not <DAV:no-lock>)");
        let lists = parse_if_header(&headers);
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].conditions.len(), 1);
        assert_eq!(
            lists[0].conditions[0],
            IfCondition::Not(Box::new(IfCondition::StateToken("DAV:no-lock".into())))
        );
    }

    #[test]
    fn test_parse_if_resource_tag() {
        let headers = make_if_header("</path> (<opaquelocktoken:t1>)");
        let lists = parse_if_header(&headers);
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].resource_tag, Some("/path".into()));
        assert_eq!(lists[0].conditions.len(), 1);
    }

    #[test]
    fn test_parse_if_and_conditions() {
        let headers = make_if_header("(<opaquelocktoken:a> <opaquelocktoken:b>)");
        let lists = parse_if_header(&headers);
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].conditions.len(), 2);
    }

    #[test]
    fn test_parse_if_multiple_lists() {
        let headers = make_if_header("(<opaquelocktoken:a>) (Not <DAV:no-lock>)");
        let lists = parse_if_header(&headers);
        assert_eq!(lists.len(), 2);
    }

    #[test]
    fn test_parse_if_no_header() {
        let headers = HeaderMap::new();
        let lists = parse_if_header(&headers);
        assert!(lists.is_empty());
    }

    #[test]
    fn test_positive_tokens() {
        let lists = [IfList {
            resource_tag: None,
            conditions: vec![
                IfCondition::StateToken("t1".into()),
                IfCondition::Not(Box::new(IfCondition::StateToken("t2".into()))),
                IfCondition::StateToken("t3".into()),
            ],
        }];
        let tokens = lists[0].positive_tokens();
        assert_eq!(tokens, vec!["t1", "t3"]);
    }
}
