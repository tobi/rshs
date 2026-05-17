pub mod fs;
pub mod xml;

use std::collections::HashMap;
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

#[derive(Debug)]
pub enum PropRequest {
    AllProp,
    PropName,
    Named(Vec<String>),
}

pub struct PropEntry {
    pub href: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: SystemTime,
    pub created: Option<SystemTime>,
    pub content_type: Option<String>,
    pub dead_props: Option<HashMap<String, String>>,
    pub canonical_path: Option<PathBuf>,
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
}

#[derive(Debug, Clone)]
pub enum LockScope {
    Exclusive,
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

pub fn parse_if_header(headers: &HeaderMap) -> Vec<String> {
    let mut tokens = Vec::new();
    let value = match headers.get("if").and_then(|v| v.to_str().ok()) {
        Some(v) => v,
        None => return tokens,
    };
    for part in value.split('(') {
        let inner = part.trim_end_matches(')').trim();
        let token = inner.trim_matches('<').trim_matches('>').trim();
        if !token.is_empty() {
            tokens.push(token.to_string());
        }
    }
    tokens
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

pub fn parse_propfind_request(xml: &[u8]) -> Result<PropRequest, Box<dyn std::error::Error>> {
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

pub fn parse_proppatch_request(xml: &[u8]) -> Result<PropPatchOp, Box<dyn std::error::Error>> {
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
        return Err("invalid PROPPATCH body".into());
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
}
