use std::io::Cursor;
use std::time::UNIX_EPOCH;

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
    let mut w = |ev: Event<'_>| {
        writer.write_event(ev).unwrap();
    };

    w(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}propstat"
    ))));
    w(Event::Start(BytesStart::new(format!("{DAV_PREFIX}prop"))));

    for prop_name in props {
        match **prop_name {
            "creationdate" => {
                let date = entry
                    .created
                    .map(crate::utils::time::format_rfc3339)
                    .unwrap_or_default();
                w(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}creationdate"
                ))));
                w(Event::Text(BytesText::new(&date)));
                w(Event::End(BytesEnd::new(format!(
                    "{DAV_PREFIX}creationdate"
                ))));
            }
            "getcontentlength" => {
                w(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}getcontentlength"
                ))));
                w(Event::Text(BytesText::new(&entry.size.to_string())));
                w(Event::End(BytesEnd::new(format!(
                    "{DAV_PREFIX}getcontentlength"
                ))));
            }
            "getcontenttype" => {
                let ct = entry.content_type.as_deref().unwrap_or("");
                w(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}getcontenttype"
                ))));
                w(Event::Text(BytesText::new(ct)));
                w(Event::End(BytesEnd::new(format!(
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
                w(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}getetag"
                ))));
                w(Event::Text(BytesText::new(&etag)));
                w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}getetag"))));
            }
            "getlastmodified" => {
                w(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}getlastmodified"
                ))));
                let date = crate::utils::time::format_rfc1123(entry.modified);
                w(Event::Text(BytesText::new(&date)));
                w(Event::End(BytesEnd::new(format!(
                    "{DAV_PREFIX}getlastmodified"
                ))));
            }
            "lockdiscovery" => {
                w(Event::Empty(BytesStart::new(format!(
                    "{DAV_PREFIX}lockdiscovery"
                ))));
            }
            "resourcetype" => {
                w(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}resourcetype"
                ))));
                if entry.is_dir {
                    w(Event::Empty(BytesStart::new(format!(
                        "{DAV_PREFIX}collection"
                    ))));
                }
                w(Event::End(BytesEnd::new(format!(
                    "{DAV_PREFIX}resourcetype"
                ))));
            }
            "supportedlock" => {
                w(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}supportedlock"
                ))));
                write_lockentry(&mut w, "exclusive");
                write_lockentry(&mut w, "shared");
                w(Event::End(BytesEnd::new(format!(
                    "{DAV_PREFIX}supportedlock"
                ))));
            }
            _ => {}
        }
    }

    w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}prop"))));
    w(Event::Start(BytesStart::new(format!("{DAV_PREFIX}status"))));
    w(Event::Text(BytesText::new("HTTP/1.1 200 OK")));
    w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}status"))));
    w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}propstat"))));
}

fn write_lockentry(w: &mut impl FnMut(Event<'_>), scope: &str) {
    w(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}lockentry"
    ))));
    w(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}lockscope"
    ))));
    w(Event::Empty(BytesStart::new(format!(
        "{DAV_PREFIX}{scope}"
    ))));
    w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}lockscope"))));
    w(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}locktype"
    ))));
    w(Event::Empty(BytesStart::new(format!("{DAV_PREFIX}write"))));
    w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}locktype"))));
    w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}lockentry"))));
}

fn write_propstat_404(writer: &mut Writer<Cursor<Vec<u8>>>, props: &[String]) {
    let mut w = |ev: Event<'_>| {
        writer.write_event(ev).unwrap();
    };

    w(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}propstat"
    ))));
    w(Event::Start(BytesStart::new(format!("{DAV_PREFIX}prop"))));

    for prop_name in props {
        w(Event::Empty(BytesStart::new(format!(
            "{DAV_PREFIX}{prop_name}"
        ))));
    }

    w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}prop"))));
    w(Event::Start(BytesStart::new(format!("{DAV_PREFIX}status"))));
    w(Event::Text(BytesText::new("HTTP/1.1 404 Not Found")));
    w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}status"))));
    w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}propstat"))));
}

fn write_propname(writer: &mut Writer<Cursor<Vec<u8>>>, props: &[&str]) {
    let mut w = |ev: Event<'_>| {
        writer.write_event(ev).unwrap();
    };

    w(Event::Start(BytesStart::new(format!(
        "{DAV_PREFIX}propstat"
    ))));
    w(Event::Start(BytesStart::new(format!("{DAV_PREFIX}prop"))));

    for prop_name in props {
        w(Event::Empty(BytesStart::new(format!(
            "{DAV_PREFIX}{prop_name}"
        ))));
    }

    w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}prop"))));
    w(Event::Start(BytesStart::new(format!("{DAV_PREFIX}status"))));
    w(Event::Text(BytesText::new("HTTP/1.1 200 OK")));
    w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}status"))));
    w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}propstat"))));
}
