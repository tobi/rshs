use std::io::Cursor;
use std::time::UNIX_EPOCH;

use axum::{body::Body, http::StatusCode, response::Response};
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};

use crate::webdav::{PropEntry, PropRequest};

pub const DAV_PREFIX: &str = "D:";
const DAV_NS: &str = "DAV:";

trait XmlWriterExt {
    fn ev(&mut self, event: Event<'_>);
}

impl XmlWriterExt for Writer<Cursor<Vec<u8>>> {
    fn ev(&mut self, event: Event<'_>) {
        self.write_event(event).unwrap();
    }
}

/// Build a `207 Multi-Status` XML response.
pub fn multistatus(xml: String) -> Response {
    xml_response(StatusCode::from_u16(207).unwrap(), xml)
}

/// Build an XML response with the given status code.
pub fn xml_response(status: StatusCode, xml: String) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "application/xml; charset=utf-8")
        .body(Body::from(xml))
        .unwrap()
}

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

pub fn build_multistatus(entries: &[PropEntry], prop_request: &PropRequest) -> String {
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    writer
        .write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)))
        .unwrap();

    let mut ms = BytesStart::new(format!("{DAV_PREFIX}multistatus"));
    ms.push_attribute(("xmlns:D", DAV_NS));
    writer.write_event(Event::Start(ms)).unwrap();

    for entry in entries {
        write_response(&mut writer, entry, prop_request);
    }

    writer
        .write_event(Event::End(BytesEnd::new(format!(
            "{DAV_PREFIX}multistatus"
        ))))
        .unwrap();

    String::from_utf8(writer.into_inner().into_inner()).unwrap()
}

fn write_response(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    entry: &PropEntry,
    prop_request: &PropRequest,
) {
    writer
        .write_event(Event::Start(BytesStart::new(format!(
            "{DAV_PREFIX}response"
        ))))
        .unwrap();

    writer
        .write_event(Event::Start(BytesStart::new(format!("{DAV_PREFIX}href"))))
        .unwrap();
    writer
        .write_event(Event::Text(BytesText::new(&entry.href)))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}href"))))
        .unwrap();

    if matches!(prop_request, PropRequest::PropName) {
        write_propname(writer, SUPPORTED_PROPS);
        writer
            .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}response"))))
            .unwrap();
        return;
    }

    let (found, missing) = match prop_request {
        PropRequest::AllProp => {
            let all: Vec<&str> = SUPPORTED_PROPS.to_vec();
            (all, vec![])
        }
        PropRequest::Named(names) => {
            let found: Vec<&str> = SUPPORTED_PROPS
                .iter()
                .filter(|p| names.iter().any(|n| n == **p))
                .copied()
                .collect();
            let missing: Vec<&str> = names
                .iter()
                .filter(|n| !SUPPORTED_PROPS.contains(&n.as_str()))
                .map(|s| s.as_str())
                .collect();
            (found, missing)
        }
        PropRequest::PropName => unreachable!(),
    };

    let applicable: Vec<&&str> = found
        .iter()
        .filter(|p| is_applicable(p, entry.is_dir))
        .collect();

    if !applicable.is_empty() {
        write_propstat_200(writer, entry, &applicable);
    }

    let mut not_found: Vec<String> = found
        .iter()
        .filter(|p| !is_applicable(p, entry.is_dir))
        .map(|p| p.to_string())
        .collect();
    not_found.extend(missing.iter().map(|p| p.to_string()));

    if !not_found.is_empty() {
        write_propstat_404(writer, &not_found);
    }

    // Dead properties
    if let Some(ref dead) = entry.dead_props {
        if !dead.is_empty() {
            write_dead_propstat(writer, dead);
        }
    }

    writer
        .write_event(Event::End(BytesEnd::new(format!("{DAV_PREFIX}response"))))
        .unwrap();
}

fn is_applicable(prop: &str, is_dir: bool) -> bool {
    match prop {
        "getcontentlength" | "getcontenttype" | "getetag" => !is_dir,
        _ => true,
    }
}

fn write_propstat_200(writer: &mut Writer<Cursor<Vec<u8>>>, entry: &PropEntry, props: &[&&str]) {
    writer.ev(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}propstat"
    ))));
    writer.ev(Event::Start(BytesStart::new(format!("{DAV_PREFIX}prop"))));

    for prop_name in props {
        match **prop_name {
            "creationdate" => {
                let date = entry
                    .created
                    .map(crate::utils::time::format_rfc3339)
                    .unwrap_or_default();
                writer.ev(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}creationdate"
                ))));
                writer.ev(Event::Text(BytesText::new(&date)));
                writer.ev(Event::End(BytesEnd::new(format!(
                    "{DAV_PREFIX}creationdate"
                ))));
            }
            "getcontentlength" => {
                writer.ev(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}getcontentlength"
                ))));
                writer.ev(Event::Text(BytesText::new(&entry.size.to_string())));
                writer.ev(Event::End(BytesEnd::new(format!(
                    "{DAV_PREFIX}getcontentlength"
                ))));
            }
            "getcontenttype" => {
                let ct = entry.content_type.as_deref().unwrap_or("");
                writer.ev(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}getcontenttype"
                ))));
                writer.ev(Event::Text(BytesText::new(ct)));
                writer.ev(Event::End(BytesEnd::new(format!(
                    "{DAV_PREFIX}getcontenttype"
                ))));
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
                writer.ev(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}getetag"
                ))));
                writer.ev(Event::Text(BytesText::new(&etag)));
                writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}getetag"))));
            }
            "getlastmodified" => {
                writer.ev(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}getlastmodified"
                ))));
                let date = crate::utils::time::format_rfc1123(entry.modified);
                writer.ev(Event::Text(BytesText::new(&date)));
                writer.ev(Event::End(BytesEnd::new(format!(
                    "{DAV_PREFIX}getlastmodified"
                ))));
            }
            "lockdiscovery" => {
                writer.ev(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}lockdiscovery"
                ))));
                if let Some(ref locks) = entry.active_locks {
                    for lock in locks {
                        write_activelock(writer, lock);
                    }
                }
                writer.ev(Event::End(BytesEnd::new(format!(
                    "{DAV_PREFIX}lockdiscovery"
                ))));
            }
            "resourcetype" => {
                writer.ev(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}resourcetype"
                ))));
                if entry.is_dir {
                    writer.ev(Event::Empty(BytesStart::new(format!(
                        "{DAV_PREFIX}collection"
                    ))));
                }
                writer.ev(Event::End(BytesEnd::new(format!(
                    "{DAV_PREFIX}resourcetype"
                ))));
            }
            "supportedlock" => {
                writer.ev(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}supportedlock"
                ))));
                write_lockentry(writer, "exclusive");
                write_lockentry(writer, "shared");
                writer.ev(Event::End(BytesEnd::new(format!(
                    "{DAV_PREFIX}supportedlock"
                ))));
            }
            _ => {}
        }
    }

    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}prop"))));
    writer.ev(Event::Start(BytesStart::new(format!("{DAV_PREFIX}status"))));
    writer.ev(Event::Text(BytesText::new("HTTP/1.1 200 OK")));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}status"))));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}propstat"))));
}

fn write_lockentry(writer: &mut Writer<Cursor<Vec<u8>>>, scope: &str) {
    writer.ev(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}lockentry"
    ))));
    writer.ev(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}lockscope"
    ))));
    writer.ev(Event::Empty(BytesStart::new(format!(
        "{DAV_PREFIX}{scope}"
    ))));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}lockscope"))));
    writer.ev(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}locktype"
    ))));
    writer.ev(Event::Empty(BytesStart::new(format!("{DAV_PREFIX}write"))));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}locktype"))));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}lockentry"))));
}

fn write_propstat_404(writer: &mut Writer<Cursor<Vec<u8>>>, props: &[String]) {
    writer.ev(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}propstat"
    ))));
    writer.ev(Event::Start(BytesStart::new(format!("{DAV_PREFIX}prop"))));

    for prop_name in props {
        writer.ev(Event::Empty(BytesStart::new(format!(
            "{DAV_PREFIX}{prop_name}"
        ))));
    }

    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}prop"))));
    writer.ev(Event::Start(BytesStart::new(format!("{DAV_PREFIX}status"))));
    writer.ev(Event::Text(BytesText::new("HTTP/1.1 404 Not Found")));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}status"))));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}propstat"))));
}

fn write_propname(writer: &mut Writer<Cursor<Vec<u8>>>, props: &[&str]) {
    writer.ev(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}propstat"
    ))));
    writer.ev(Event::Start(BytesStart::new(format!("{DAV_PREFIX}prop"))));

    for prop_name in props {
        writer.ev(Event::Empty(BytesStart::new(format!(
            "{DAV_PREFIX}{prop_name}"
        ))));
    }

    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}prop"))));
    writer.ev(Event::Start(BytesStart::new(format!("{DAV_PREFIX}status"))));
    writer.ev(Event::Text(BytesText::new("HTTP/1.1 200 OK")));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}status"))));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}propstat"))));
}

fn write_activelock(writer: &mut Writer<Cursor<Vec<u8>>>, lock: &super::LockInfo) {
    writer.ev(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}activelock"
    ))));

    writer.ev(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}lockscope"
    ))));
    writer.ev(Event::Empty(BytesStart::new(format!(
        "{DAV_PREFIX}exclusive"
    ))));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}lockscope"))));

    writer.ev(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}locktype"
    ))));
    writer.ev(Event::Empty(BytesStart::new(format!("{DAV_PREFIX}write"))));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}locktype"))));

    writer.ev(Event::Start(BytesStart::new(format!("{DAV_PREFIX}depth"))));
    writer.ev(Event::Text(BytesText::new("0")));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}depth"))));

    if let Some(ref owner) = lock.owner {
        writer.ev(Event::Start(BytesStart::new(format!("{DAV_PREFIX}owner"))));
        writer.ev(Event::Text(BytesText::new(owner)));
        writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}owner"))));
    }

    if let Some(d) = lock.timeout {
        writer.ev(Event::Start(BytesStart::new(format!(
            "{DAV_PREFIX}timeout"
        ))));
        writer.ev(Event::Text(BytesText::new(&format!(
            "Second-{}",
            d.as_secs()
        ))));
        writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}timeout"))));
    }

    writer.ev(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}locktoken"
    ))));
    writer.ev(Event::Start(BytesStart::new(format!("{DAV_PREFIX}href"))));
    writer.ev(Event::Text(BytesText::new(&lock.token)));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}href"))));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}locktoken"))));

    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}activelock"))));
}

fn write_dead_propstat(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    props: &std::collections::HashMap<String, String>,
) {
    writer.ev(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}propstat"
    ))));
    writer.ev(Event::Start(BytesStart::new(format!("{DAV_PREFIX}prop"))));

    for (name, value) in props {
        writer.ev(Event::Start(BytesStart::new(name.as_str())));
        writer.ev(Event::Text(BytesText::new(value)));
        writer.ev(Event::End(BytesEnd::new(name.as_str())));
    }

    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}prop"))));
    writer.ev(Event::Start(BytesStart::new(format!("{DAV_PREFIX}status"))));
    writer.ev(Event::Text(BytesText::new("HTTP/1.1 200 OK")));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}status"))));
    writer.ev(Event::End(BytesEnd::new(format!("{DAV_PREFIX}propstat"))));
}
