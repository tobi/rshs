use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};

use super::{PropEntry, PropRequest};

pub const DAV_PREFIX: &str = "D:";
const DAV_NS: &str = "DAV:";

const SUPPORTED_PROPS: &[&str] = &["getcontentlength", "getlastmodified", "resourcetype"];

pub fn build_multistatus(entries: &[PropEntry], prop_request: &PropRequest) -> String {
    let mut writer = Writer::new_with_indent(Cursor::new(Vec::new()), b' ', 2);

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
        "getcontentlength" => !is_dir,
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
            "getcontentlength" => {
                w(Event::Start(BytesStart::new(format!(
                    "{DAV_PREFIX}getcontentlength"
                ))));
                w(Event::Text(BytesText::new(&entry.size.to_string())));
                w(Event::End(BytesEnd::new(format!(
                    "{DAV_PREFIX}getcontentlength"
                ))));
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
            _ => {}
        }
    }

    w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}prop"))));
    w(Event::Start(BytesStart::new(format!("{DAV_PREFIX}status"))));
    w(Event::Text(BytesText::new("HTTP/1.1 200 OK")));
    w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}status"))));
    w(Event::End(BytesEnd::new(format!("{DAV_PREFIX}propstat"))));
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
