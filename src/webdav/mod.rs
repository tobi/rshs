pub mod fs;
pub mod xml;

use std::sync::LazyLock;
use std::time::SystemTime;

use axum::http::{HeaderMap, Method as HttpMethod};
use percent_encoding::percent_decode_str;
use quick_xml::Reader;
use quick_xml::events::Event;

type Method = LazyLock<HttpMethod>;

pub static M_PROPFIND: Method = LazyLock::new(|| HttpMethod::from_bytes(b"PROPFIND").unwrap());
pub static M_MKCOL: Method = LazyLock::new(|| HttpMethod::from_bytes(b"MKCOL").unwrap());
pub static M_COPY: Method = LazyLock::new(|| HttpMethod::from_bytes(b"COPY").unwrap());
pub static M_MOVE: Method = LazyLock::new(|| HttpMethod::from_bytes(b"MOVE").unwrap());
pub static _M_LOCK: Method = LazyLock::new(|| HttpMethod::from_bytes(b"LOCK").unwrap());
pub static _M_UNLOCK: Method = LazyLock::new(|| HttpMethod::from_bytes(b"UNLOCK").unwrap());

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
pub fn parse_destination(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("destination")?.to_str().ok()?;
    if let Some(pos) = value.find("://") {
        let after_scheme = &value[pos + 3..];
        if let Some(slash_pos) = after_scheme.find('/') {
            let path = &after_scheme[slash_pos..];
            return Some(percent_decode_str(path).decode_utf8_lossy().to_string());
        }
    }
    if value.starts_with('/') {
        return Some(percent_decode_str(value).decode_utf8_lossy().to_string());
    }
    None
}

pub fn parse_overwrite(headers: &HeaderMap) -> bool {
    headers
        .get("overwrite")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_ascii_uppercase())
        .unwrap_or_else(|| "T".into())
        != "F"
}
