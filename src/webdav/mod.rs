pub mod fs;
pub mod ls;
pub mod method;
pub mod xml;

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::http::HeaderMap;
use percent_encoding::percent_decode_str;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

pub use method::Method;

pub type DeadPropertyStore = HashMap<PathBuf, HashMap<String, String>>;

#[derive(Debug, Clone)]
pub struct PropPatchAction {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PropPatchOp {
    pub actions: Vec<PropPatchAction>,
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

impl PropEntry {
    pub fn new(
        href: String,
        is_dir: bool,
        size: u64,
        modified: SystemTime,
        created: Option<SystemTime>,
    ) -> Self {
        Self {
            canonical_path: None,
            content_type: None,
            dead_props: None,
            active_locks: None,
            href,
            modified,
            created,
            size,
            is_dir,
        }
    }

    pub fn from_meta(href: String, is_dir: bool, meta: &std::fs::Metadata) -> Self {
        Self::new(
            href,
            is_dir,
            meta.len(),
            meta.modified().unwrap_or(UNIX_EPOCH),
            meta.created().ok(),
        )
    }
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
        self.positive_tokens_iter().collect()
    }

    pub fn positive_tokens_iter(&self) -> impl Iterator<Item = &str> + '_ {
        self.conditions.iter().filter_map(|c| match c {
            IfCondition::StateToken(t) => Some(t.as_str()),
            _ => None,
        })
    }

    pub fn has_lock_token(&self) -> bool {
        self.conditions.iter().any(|c| match c {
            IfCondition::StateToken(t) => t != "DAV:no-lock",
            IfCondition::Not(inner) => {
                matches!(inner.as_ref(), IfCondition::StateToken(t) if t != "DAV:no-lock")
            }
        })
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

pub use ls::{find_ancestor_lock, walk_locked_ancestors};

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

pub fn clark_key(ns: &str, local: &str) -> String {
    if ns.is_empty() {
        local.to_string()
    } else {
        format!("{{{}}}{}", ns, local)
    }
}

pub fn parse_clark(key: &str) -> Option<(&str, &str)> {
    if let Some(rest) = key.strip_prefix('{') {
        let (ns, local) = rest.split_once('}')?;
        Some((ns, local))
    } else {
        Some(("", key))
    }
}

fn extract_element_ns(e: &BytesStart) -> Result<(String, String), ParseError> {
    let qname = e.name();
    let name = qname.as_ref();
    let (prefix, local) = match name.iter().position(|&b| b == b':') {
        Some(pos) => (
            Some(String::from_utf8_lossy(&name[..pos]).to_string()),
            String::from_utf8_lossy(&name[pos + 1..]).to_string(),
        ),
        None => (None, String::from_utf8_lossy(name).to_string()),
    };
    let ns = match prefix {
        Some(ref p) => {
            let key = format!("xmlns:{}", p);
            let attr = e
                .attributes()
                .flatten()
                .find(|a| String::from_utf8_lossy(a.key.as_ref()) == key);
            match attr {
                Some(a) => {
                    let value = String::from_utf8_lossy(&a.value);
                    if value.is_empty() {
                        return Err(ParseError::InvalidBody(
                            "invalid namespace declaration: empty URI",
                        ));
                    }
                    value.to_string()
                }
                None => String::new(),
            }
        }
        None => e
            .attributes()
            .flatten()
            .find(|a| a.key.as_ref() == b"xmlns")
            .map(|a| String::from_utf8_lossy(&a.value).to_string())
            .unwrap_or_default(),
    };
    Ok((ns, local))
}

pub fn parse_propfind_request(xml: &[u8]) -> Result<PropRequest, ParseError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);

    let mut props = Vec::new();
    let mut in_prop = false;
    let mut found_allprop = false;
    let mut found_propname = false;
    let mut seen_element = false;

    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                seen_element = true;
                let (ns, local) = extract_element_ns(&e)?;
                let name = local.as_bytes();
                match name {
                    b"prop" => in_prop = true,
                    b"allprop" => found_allprop = true,
                    b"propname" => found_propname = true,
                    _ if in_prop => {
                        props.push(clark_key(&ns, &local));
                    }
                    _ => {}
                }
            }
            Event::End(e) => {
                let local_name = e.local_name();
                let local = String::from_utf8_lossy(local_name.as_ref());
                if local == "prop" {
                    in_prop = false;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if found_allprop || (props.is_empty() && !seen_element) {
        Ok(PropRequest::AllProp)
    } else if props.is_empty() && seen_element {
        Err(ParseError::InvalidBody("invalid PROPFIND request body"))
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

fn decode_xml_char_refs(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("&#") {
        result.push_str(&rest[..pos]);
        rest = &rest[pos + 2..];
        let hex = rest.starts_with('x') || rest.starts_with('X');
        if hex {
            rest = &rest[1..];
        }
        if let Some(end) = rest.find(';') {
            let num_str = &rest[..end];
            let radix = if hex { 16 } else { 10 };
            if let Ok(n) = u32::from_str_radix(num_str, radix) {
                if let Some(c) = char::from_u32(n) {
                    result.push(c);
                }
            }
            rest = &rest[end + 1..];
        } else {
            break;
        }
    }
    result.push_str(rest);
    result
}

pub fn parse_proppatch_request(xml: &[u8]) -> Result<PropPatchOp, ParseError> {
    // Pre-decode XML character references: quick_xml 0.40 does not emit
    // Text events for content that is entirely character references.
    let decoded = decode_xml_char_refs(&String::from_utf8_lossy(xml));
    let mut reader = Reader::from_reader(decoded.as_bytes());
    reader.config_mut().trim_text(true);

    let mut actions = Vec::new();
    let mut in_set = false;
    let mut in_remove = false;
    let mut current_name: Option<String> = None;

    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                let (ns, local) = extract_element_ns(&e)?;
                match &*local {
                    "set" => in_set = true,
                    "remove" => in_remove = true,
                    "prop" => {}
                    _ if in_set => {
                        current_name = Some(clark_key(&ns, &local));
                    }
                    _ if in_remove => {
                        actions.push(PropPatchAction {
                            name: clark_key(&ns, &local),
                            value: None,
                        });
                    }
                    _ => {}
                }
            }
            Event::Empty(e) => {
                let (ns, local) = extract_element_ns(&e)?;
                if in_remove && local != "prop" {
                    actions.push(PropPatchAction {
                        name: clark_key(&ns, &local),
                        value: None,
                    });
                } else if in_set && local != "prop" {
                    actions.push(PropPatchAction {
                        name: clark_key(&ns, &local),
                        value: Some(String::new()),
                    });
                }
            }
            Event::Text(t) if in_set && current_name.is_some() => {
                let raw = String::from_utf8_lossy(t.as_ref());
                let val = decode_xml_char_refs(&raw);
                actions.push(PropPatchAction {
                    name: current_name.take().unwrap(),
                    value: Some(val),
                });
            }
            Event::End(e) => {
                let local_name = e.local_name();
                let local = String::from_utf8_lossy(local_name.as_ref());
                match &*local {
                    "set" => in_set = false,
                    "remove" => in_remove = false,
                    _ if in_set && current_name.is_some() => {
                        actions.push(PropPatchAction {
                            name: current_name.take().unwrap(),
                            value: Some(String::new()),
                        });
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if actions.is_empty() {
        return Err(ParseError::InvalidBody("invalid PROPPATCH body"));
    }

    Ok(PropPatchOp { actions })
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
        let list = IfList {
            resource_tag: None,
            conditions: vec![
                IfCondition::StateToken("t1".into()),
                IfCondition::Not(Box::new(IfCondition::StateToken("t2".into()))),
                IfCondition::StateToken("t3".into()),
            ],
        };
        let tokens = list.positive_tokens();
        assert_eq!(tokens, vec!["t1", "t3"]);
    }

    #[test]
    fn test_positive_tokens_iter() {
        let list = IfList {
            resource_tag: None,
            conditions: vec![
                IfCondition::StateToken("t1".into()),
                IfCondition::Not(Box::new(IfCondition::StateToken("t2".into()))),
                IfCondition::StateToken("t3".into()),
            ],
        };
        let tokens = list.positive_tokens_iter();
        assert_eq!(tokens.collect::<Vec<_>>(), vec!["t1", "t3"]);
    }

    #[test]
    fn test_has_lock_token_with_lock_token() {
        let list = IfList {
            resource_tag: None,
            conditions: vec![IfCondition::StateToken("opaquelocktoken:abc".into())],
        };
        assert!(list.has_lock_token());
    }

    #[test]
    fn test_has_lock_token_dav_no_lock_only() {
        let list = IfList {
            resource_tag: None,
            conditions: vec![IfCondition::StateToken("DAV:no-lock".into())],
        };
        assert!(!list.has_lock_token());
    }

    #[test]
    fn test_has_lock_token_not_dav_no_lock() {
        let list = IfList {
            resource_tag: None,
            conditions: vec![IfCondition::Not(Box::new(IfCondition::StateToken(
                "DAV:no-lock".into(),
            )))],
        };
        assert!(!list.has_lock_token());
    }

    #[test]
    fn test_has_lock_token_not_lock_token() {
        let list = IfList {
            resource_tag: None,
            conditions: vec![IfCondition::Not(Box::new(IfCondition::StateToken(
                "opaquelocktoken:abc".into(),
            )))],
        };
        assert!(list.has_lock_token());
    }

    #[test]
    fn test_has_lock_token_mixed() {
        let list = IfList {
            resource_tag: None,
            conditions: vec![
                IfCondition::StateToken("DAV:no-lock".into()),
                IfCondition::StateToken("opaquelocktoken:xyz".into()),
            ],
        };
        assert!(list.has_lock_token());
    }

    #[test]
    fn test_clark_key_with_ns() {
        assert_eq!(
            clark_key("http://example.com", "prop0"),
            "{http://example.com}prop0"
        );
    }

    #[test]
    fn test_clark_key_empty_ns() {
        assert_eq!(clark_key("", "prop0"), "prop0");
    }

    #[test]
    fn test_parse_clark_with_ns() {
        assert_eq!(
            parse_clark("{http://example.com}prop0"),
            Some(("http://example.com", "prop0"))
        );
    }

    #[test]
    fn test_parse_clark_empty_ns() {
        assert_eq!(parse_clark("prop0"), Some(("", "prop0")));
    }

    #[test]
    fn test_parse_clark_invalid() {
        assert_eq!(parse_clark("{broken"), None);
    }

    #[test]
    fn test_extract_element_ns_default_ns() {
        let mut elem = BytesStart::new("prop0");
        elem.push_attribute(("xmlns", "http://example.com/neon/litmus/"));
        let (ns, local) = extract_element_ns(&elem).unwrap();
        assert_eq!(ns, "http://example.com/neon/litmus/");
        assert_eq!(local, "prop0");
    }

    #[test]
    fn test_extract_element_ns_no_ns() {
        let elem = BytesStart::new("prop0");
        let (ns, local) = extract_element_ns(&elem).unwrap();
        assert_eq!(ns, "");
        assert_eq!(local, "prop0");
    }

    #[test]
    fn test_extract_element_ns_prefixed() {
        let mut elem = BytesStart::new("X:prop0");
        elem.push_attribute(("xmlns:X", "http://example.com/ns"));
        let (ns, local) = extract_element_ns(&elem).unwrap();
        assert_eq!(ns, "http://example.com/ns");
        assert_eq!(local, "prop0");
    }

    #[test]
    fn test_extract_element_ns_invalid_empty_uri() {
        let mut elem = BytesStart::new("bar:foo");
        elem.push_attribute(("xmlns:bar", ""));
        assert!(extract_element_ns(&elem).is_err());
    }

    #[test]
    fn test_parse_propfind_invalid_xml_returns_error() {
        let result = parse_propfind_request(b"<foo>");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_proppatch_preserves_namespace() {
        let xml = br#"<?xml version="1.0" encoding="utf-8"?><D:propertyupdate xmlns:D="DAV:"><D:set><D:prop><prop0 xmlns="http://example.com/neon/litmus/">value0</prop0></D:prop></D:set></D:propertyupdate>"#;
        let op = parse_proppatch_request(xml).unwrap();
        let value = op.actions.iter().find_map(|a| match a {
            PropPatchAction {
                name,
                value: Some(v),
            } if name == "{http://example.com/neon/litmus/}prop0" => Some(v.as_str()),
            _ => None,
        });
        assert_eq!(value, Some("value0"));
    }

    #[test]
    fn test_parse_proppatch_respects_order_set_then_remove() {
        let xml = br#"<?xml version="1.0"?><D:propertyupdate xmlns:D="DAV:"><D:set><D:prop><X:p>val</X:p></D:prop></D:set><D:remove><D:prop><X:p/></D:prop></D:remove></D:propertyupdate>"#;
        let op = parse_proppatch_request(xml).unwrap();
        assert_eq!(op.actions.len(), 2);
        assert_eq!(op.actions[0].name, "p");
        assert_eq!(op.actions[0].value.as_deref(), Some("val"));
        assert_eq!(op.actions[1].name, "p");
        assert!(op.actions[1].value.is_none());
    }

    #[test]
    fn test_parse_proppatch_high_unicode_character() {
        let xml = br#"<?xml version="1.0" encoding="utf-8" ?><propertyupdate xmlns='DAV:'><set><prop><high-unicode xmlns='http://example.com/neon/litmus/'>&#65536;</high-unicode></prop></set></propertyupdate>"#;
        let op = parse_proppatch_request(xml).unwrap();
        let value = op.actions.iter().find_map(|a| match a {
            PropPatchAction {
                name,
                value: Some(v),
            } if name == "{http://example.com/neon/litmus/}high-unicode" => Some(v.as_str()),
            _ => None,
        });
        assert_eq!(value, Some("𐀀"), "high-unicode value should be U+10000 (𐀀)");

        // Also test basic char ref
        let xml = br#"<?xml version="1.0"?><D:propertyupdate xmlns:D="DAV:"><D:set><D:prop><X:p>&#65;&#66;&#67;</X:p></D:prop></D:set></D:propertyupdate>"#;
        let op = parse_proppatch_request(xml).unwrap();
        let val = op.actions.iter().find_map(|a| match a {
            PropPatchAction {
                name,
                value: Some(v),
            } if name == "p" => Some(v.as_str()),
            _ => None,
        });
        assert_eq!(val, Some("ABC"), "&#65;&#66;&#67; should decode to ABC");
    }
}
