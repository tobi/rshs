use std::io::Cursor;
use std::time::UNIX_EPOCH;

use axum::{body::Body, http::StatusCode, response::Response};
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};

use super::{PropEntry, PropRequest};

pub const DAV_PREFIX: &str = "D:";

const DAV_NS: &str = "DAV:";
const SUPPORTED_PROPS: &[&str] = &[
    "creationdate",
    "getcontentlength",
    "getcontenttype",
    "getetag",
    "getlastmodified",
    "lockdiscovery",
    "resourcetype",
    "supportedlock",
];

pub fn dav_qname(name: &str) -> String {
    format!("{DAV_PREFIX}{name}")
}

pub type XmlWriter = Writer<Cursor<Vec<u8>>>;

pub trait XmlWriterExt {
    fn ev(&mut self, event: Event<'_>);
}

impl XmlWriterExt for XmlWriter {
    fn ev(&mut self, event: Event<'_>) {
        self.write_event(event).unwrap();
    }
}

/// Build a `207 Multi-Status` XML response.
pub fn multistatus(xml: String) -> Response {
    xml_response(StatusCode::from_u16(207).unwrap(), xml)
}

/// Build an XML response with the given status code.
fn xml_response(status: StatusCode, xml: String) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "application/xml; charset=utf-8")
        .body(Body::from(xml))
        .unwrap()
}

pub fn build_multistatus(entries: &[PropEntry], prop_request: &PropRequest) -> String {
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    writer.ev(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)));

    let mut ms = BytesStart::new(dav_qname("multistatus"));
    ms.push_attribute(("xmlns:D", DAV_NS));
    writer.ev(Event::Start(ms));

    for entry in entries {
        write_response(&mut writer, entry, prop_request);
    }

    writer.ev(Event::End(BytesEnd::new(dav_qname("multistatus"))));

    String::from_utf8(writer.into_inner().into_inner()).unwrap()
}

fn write_response(writer: &mut XmlWriter, entry: &PropEntry, prop_request: &PropRequest) {
    writer.ev(Event::Start(BytesStart::new(dav_qname("response"))));

    writer.ev(Event::Start(BytesStart::new(dav_qname("href"))));
    writer.ev(Event::Text(BytesText::new(&entry.href)));
    writer.ev(Event::End(BytesEnd::new(dav_qname("href"))));

    if matches!(prop_request, PropRequest::PropName) {
        write_propname(writer, SUPPORTED_PROPS);
        writer.ev(Event::End(BytesEnd::new(dav_qname("response"))));
        return;
    }

    let (found, missing) = match prop_request {
        PropRequest::AllProp => (SUPPORTED_PROPS.to_vec(), vec![]),
        PropRequest::Named(names) => {
            let (mut found, mut missing) = (Vec::new(), Vec::new());

            for n in names {
                let local = super::parse_clark(n).map(|(_, l)| l).unwrap_or(n.as_str());
                if SUPPORTED_PROPS.contains(&local) {
                    if !found.contains(&local) {
                        found.push(local);
                    }
                } else {
                    missing.push(n.as_str());
                }
            }

            (found, missing)
        }
        PropRequest::PropName => unreachable!(),
    };

    let applicable = found
        .iter()
        .filter(|p| is_applicable(p, entry.is_dir))
        .copied();
    let mut applicable = applicable.peekable();

    if applicable.peek().is_some() {
        write_propstat_200(writer, entry, applicable);
    }

    let mut not_found: Vec<String> = found
        .iter()
        .filter(|p| !is_applicable(p, entry.is_dir))
        .map(|p| p.to_string())
        .collect();
    not_found.extend(missing.iter().map(|p| p.to_string()));

    // Exclude properties that have dead-prop values — they appear in the
    // 200 dead-propstat below, not here in the 404 section.
    if let Some(ref dead) = entry.dead_props {
        not_found.retain(|n| !dead.contains_key(n));
    }

    if !not_found.is_empty() {
        write_propstat_404(writer, &not_found);
    }

    // Dead properties
    if let Some(ref dead) = entry.dead_props {
        if !dead.is_empty() {
            write_dead_propstat(writer, dead);
        }
    }

    writer.ev(Event::End(BytesEnd::new(dav_qname("response"))));
}

fn is_applicable(prop: &str, is_dir: bool) -> bool {
    match prop {
        "getcontentlength" | "getcontenttype" | "getetag" => !is_dir,
        _ => true,
    }
}

fn write_prop_text(writer: &mut XmlWriter, qname: &str, value: &str) {
    writer.ev(Event::Start(BytesStart::new(qname)));
    writer.ev(Event::Text(BytesText::new(value)));
    writer.ev(Event::End(BytesEnd::new(qname)));
}

fn write_propstat_200<'a, I>(writer: &mut XmlWriter, entry: &PropEntry, props: I)
where
    I: Iterator<Item = &'a str>,
{
    writer.ev(Event::Start(BytesStart::new(dav_qname("propstat"))));
    writer.ev(Event::Start(BytesStart::new(dav_qname("prop"))));

    for prop_name in props {
        match prop_name {
            "creationdate" => {
                let date = entry
                    .created
                    .map(crate::utils::time::format_rfc3339)
                    .unwrap_or_default();
                write_prop_text(writer, &dav_qname("creationdate"), &date);
            }
            "getcontentlength" => {
                write_prop_text(
                    writer,
                    &dav_qname("getcontentlength"),
                    &entry.size.to_string(),
                );
            }
            "getcontenttype" => {
                write_prop_text(
                    writer,
                    &dav_qname("getcontenttype"),
                    entry.content_type.as_deref().unwrap_or(""),
                );
            }
            "getetag" => {
                let etag = format!(
                    "{:x}-{:x}",
                    entry
                        .modified
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                    entry.size
                );
                write_prop_text(writer, &dav_qname("getetag"), &etag);
            }
            "getlastmodified" => {
                let date = crate::utils::time::format_rfc1123(entry.modified);
                write_prop_text(writer, &dav_qname("getlastmodified"), &date);
            }
            "lockdiscovery" => {
                writer.ev(Event::Start(BytesStart::new(dav_qname("lockdiscovery"))));
                if let Some(ref locks) = entry.active_locks {
                    for lock in locks {
                        write_activelock(writer, lock);
                    }
                }
                writer.ev(Event::End(BytesEnd::new(dav_qname("lockdiscovery"))));
            }
            "resourcetype" => {
                writer.ev(Event::Start(BytesStart::new(dav_qname("resourcetype"))));
                if entry.is_dir {
                    writer.ev(Event::Empty(BytesStart::new(dav_qname("collection"))));
                }
                writer.ev(Event::End(BytesEnd::new(dav_qname("resourcetype"))));
            }
            "supportedlock" => {
                writer.ev(Event::Start(BytesStart::new(dav_qname("supportedlock"))));
                write_lockentry(writer, "exclusive");
                write_lockentry(writer, "shared");
                writer.ev(Event::End(BytesEnd::new(dav_qname("supportedlock"))));
            }
            _ => {}
        }
    }

    writer.ev(Event::End(BytesEnd::new(dav_qname("prop"))));
    writer.ev(Event::Start(BytesStart::new(dav_qname("status"))));
    writer.ev(Event::Text(BytesText::new("HTTP/1.1 200 OK")));
    writer.ev(Event::End(BytesEnd::new(dav_qname("status"))));
    writer.ev(Event::End(BytesEnd::new(dav_qname("propstat"))));
}

fn write_lockentry(writer: &mut XmlWriter, scope: &str) {
    writer.ev(Event::Start(BytesStart::new(dav_qname("lockentry"))));
    writer.ev(Event::Start(BytesStart::new(dav_qname("lockscope"))));
    writer.ev(Event::Empty(BytesStart::new(dav_qname(scope))));
    writer.ev(Event::End(BytesEnd::new(dav_qname("lockscope"))));
    writer.ev(Event::Start(BytesStart::new(dav_qname("locktype"))));
    writer.ev(Event::Empty(BytesStart::new(dav_qname("write"))));
    writer.ev(Event::End(BytesEnd::new(dav_qname("locktype"))));
    writer.ev(Event::End(BytesEnd::new(dav_qname("lockentry"))));
}

fn write_propstat_404(writer: &mut XmlWriter, props: &[String]) {
    writer.ev(Event::Start(BytesStart::new(dav_qname("propstat"))));
    writer.ev(Event::Start(BytesStart::new(dav_qname("prop"))));

    for prop_name in props {
        let (ns, local) = super::parse_clark(prop_name).unwrap_or(("", prop_name));
        let mut elem = BytesStart::new(local);
        if !ns.is_empty() {
            elem.push_attribute(("xmlns", ns));
        }
        writer.ev(Event::Empty(elem));
    }

    writer.ev(Event::End(BytesEnd::new(dav_qname("prop"))));
    writer.ev(Event::Start(BytesStart::new(dav_qname("status"))));
    writer.ev(Event::Text(BytesText::new("HTTP/1.1 404 Not Found")));
    writer.ev(Event::End(BytesEnd::new(dav_qname("status"))));
    writer.ev(Event::End(BytesEnd::new(dav_qname("propstat"))));
}

fn write_propname(writer: &mut XmlWriter, props: &[&str]) {
    writer.ev(Event::Start(BytesStart::new(dav_qname("propstat"))));
    writer.ev(Event::Start(BytesStart::new(dav_qname("prop"))));

    for prop_name in props {
        writer.ev(Event::Empty(BytesStart::new(dav_qname(prop_name))));
    }

    writer.ev(Event::End(BytesEnd::new(dav_qname("prop"))));
    writer.ev(Event::Start(BytesStart::new(dav_qname("status"))));
    writer.ev(Event::Text(BytesText::new("HTTP/1.1 200 OK")));
    writer.ev(Event::End(BytesEnd::new(dav_qname("status"))));
    writer.ev(Event::End(BytesEnd::new(dav_qname("propstat"))));
}

pub fn write_activelock(writer: &mut XmlWriter, lock: &super::LockInfo) {
    writer.ev(Event::Start(BytesStart::new(dav_qname("activelock"))));

    writer.ev(Event::Start(BytesStart::new(dav_qname("lockscope"))));
    match lock.scope {
        super::LockScope::Exclusive => {
            writer.ev(Event::Empty(BytesStart::new(dav_qname("exclusive"))));
        }
        super::LockScope::Shared => {
            writer.ev(Event::Empty(BytesStart::new(dav_qname("shared"))));
        }
    }
    writer.ev(Event::End(BytesEnd::new(dav_qname("lockscope"))));

    writer.ev(Event::Start(BytesStart::new(dav_qname("locktype"))));
    writer.ev(Event::Empty(BytesStart::new(dav_qname("write"))));
    writer.ev(Event::End(BytesEnd::new(dav_qname("locktype"))));

    let depth_str = match lock.depth {
        super::Depth::Zero => "0",
        super::Depth::One => "1",
        super::Depth::Infinity => "infinity",
    };
    writer.ev(Event::Start(BytesStart::new(dav_qname("depth"))));
    writer.ev(Event::Text(BytesText::new(depth_str)));
    writer.ev(Event::End(BytesEnd::new(dav_qname("depth"))));

    if let Some(ref owner) = lock.owner {
        writer.ev(Event::Start(BytesStart::new(dav_qname("owner"))));
        writer.ev(Event::Text(BytesText::new(owner)));
        writer.ev(Event::End(BytesEnd::new(dav_qname("owner"))));
    }

    if let Some(d) = lock.timeout {
        writer.ev(Event::Start(BytesStart::new(dav_qname("timeout"))));
        writer.ev(Event::Text(BytesText::new(&format!(
            "Second-{}",
            d.as_secs()
        ))));
        writer.ev(Event::End(BytesEnd::new(dav_qname("timeout"))));
    }

    writer.ev(Event::Start(BytesStart::new(dav_qname("locktoken"))));
    writer.ev(Event::Start(BytesStart::new(dav_qname("href"))));
    writer.ev(Event::Text(BytesText::new(&lock.token)));
    writer.ev(Event::End(BytesEnd::new(dav_qname("href"))));
    writer.ev(Event::End(BytesEnd::new(dav_qname("locktoken"))));

    writer.ev(Event::End(BytesEnd::new(dav_qname("activelock"))));
}

fn write_dead_propstat(writer: &mut XmlWriter, props: &std::collections::HashMap<String, String>) {
    writer.ev(Event::Start(BytesStart::new(dav_qname("propstat"))));
    writer.ev(Event::Start(BytesStart::new(dav_qname("prop"))));

    for (clark_key, value) in props {
        let (ns, local) = super::parse_clark(clark_key).unwrap_or(("", clark_key));
        let mut elem = BytesStart::new(local);
        if !ns.is_empty() {
            elem.push_attribute(("xmlns", ns));
        }
        writer.ev(Event::Start(elem));
        writer.ev(Event::Text(BytesText::new(value)));
        writer.ev(Event::End(BytesEnd::new(local)));
    }

    writer.ev(Event::End(BytesEnd::new(dav_qname("prop"))));
    writer.ev(Event::Start(BytesStart::new(dav_qname("status"))));
    writer.ev(Event::Text(BytesText::new("HTTP/1.1 200 OK")));
    writer.ev(Event::End(BytesEnd::new(dav_qname("status"))));
    writer.ev(Event::End(BytesEnd::new(dav_qname("propstat"))));
}
