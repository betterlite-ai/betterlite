#![cfg(test)]

use std::collections::BTreeMap;

use serde::Deserialize;

use super::frames::kind;
use crate::event_frames::beta::{BetaHeader, BetaValue, anthropic_beta_from_headers};
use crate::event_frames::sse::ServerSentEvent;

fn beta(headers: &str) -> Vec<BetaValue> {
    let map: BTreeMap<String, BetaHeader> = serde_json::from_str(headers).unwrap();
    anthropic_beta_from_headers(&map)
}

fn beta_json(headers: &str) -> String {
    serde_json::to_string(&beta(headers)).unwrap()
}

pub(super) fn anthropic_beta_header_forms() {
    assert_falsey_forms();
    assert_list_forms();
    assert_text_forms();
}

fn assert_falsey_forms() {
    for headers in [
        "{}",
        r#"{"Anthropic-Beta": "a"}"#,
        r#"{"anthropic-beta": null}"#,
        r#"{"anthropic-beta": ""}"#,
        r#"{"anthropic-beta": []}"#,
        r#"{"anthropic-beta": 0}"#,
        r#"{"anthropic-beta": false}"#,
        r#"{"anthropic-beta": {}}"#,
        r#"{"anthropic-beta": 5}"#,
        r#"{"anthropic-beta": true}"#,
        r#"{"anthropic-beta": {"a": 1}}"#,
        r#"{"anthropic-beta": "[]"}"#,
    ] {
        assert_eq!(beta(headers), Vec::<BetaValue>::new(), "{headers}");
    }
}

fn assert_list_forms() {
    assert_eq!(
        beta_json(r#"{"anthropic-beta": ["a", " a ", "", "a"]}"#),
        r#"["a"," a ","","a"]"#
    );
    assert_eq!(
        beta_json(r#"{"anthropic-beta": ["a", 1, null, true, ["x"], {"k": 1}]}"#),
        r#"["a",1,null,true,["x"],{"k":1}]"#
    );
}

fn assert_text_forms() {
    let strings = [
        ("a, b ,c", r#"["a","b","c"]"#),
        (" ", r#"[""]"#),
        ("a,,a", r#"["a","","a"]"#),
        (
            r#"[\"x\", \" y \", 5, null, true, false, [1], 1.5]"#,
            r#"["x","y","5","None","True","False","[1]","1.5"]"#,
        ),
        ("[not json]", r#"["[not json]"]"#),
        ("[a, b]", r#"["[a","b]"]"#),
        (r#" [\"a\"] "#, r#"["a"]"#),
        (r"\u001ca\u001c", r#"["a"]"#),
    ];
    for (raw, expected) in strings {
        let headers = format!(r#"{{"anthropic-beta": "{raw}"}}"#);
        assert_eq!(beta_json(&headers), expected, "{raw:?}");
    }
}

#[derive(Debug, Deserialize, PartialEq)]
struct Payload {
    id: i64,
}

pub(super) fn sse_record_and_json() {
    let blank = ServerSentEvent::new(None, None, None, None);
    assert_eq!(blank.event, None);
    assert_eq!(blank.data, "");
    assert_eq!((blank.id.as_deref(), blank.retry), (None, None));
    assert_eq!(
        blank.repr(),
        "ServerSentEvent(event=None, data=, id=None, retry=None)"
    );
    let kept = ServerSentEvent::new(Some(String::new()), None, Some(String::new()), Some(0));
    assert_eq!(kept.event, None);
    assert_eq!(kept.id.as_deref(), Some(""));
    assert_eq!(kept.retry, Some(0));
    let full = ServerSentEvent::new(
        Some("message".to_owned()),
        Some(r#"{"id":1}"#.to_owned()),
        Some("7".to_owned()),
        Some(0),
    );
    assert_eq!(
        full.repr(),
        r#"ServerSentEvent(event=message, data={"id":1}, id=7, retry=0)"#
    );
    assert_eq!(
        kept.repr(),
        "ServerSentEvent(event=None, data=, id=, retry=0)"
    );
    assert_eq!(full.json::<Payload>().unwrap(), Payload { id: 1 });
    sse_json_documents();
}

fn sse_json_documents() {
    let record = |data: &str| ServerSentEvent::new(None, Some(data.to_owned()), None, None);
    assert_eq!(record("[1, 2]").json::<Vec<i64>>().unwrap(), [1, 2]);
    assert_eq!(record("\"s\"").json::<String>().unwrap(), "s");
    assert_eq!(record("7").json::<i64>().unwrap(), 7);
    assert_eq!(record("null").json::<Option<i64>>().unwrap(), None);
    for data in ["", "{bad", r#"{"id":"#] {
        let error = record(data).json::<Payload>().unwrap_err();
        assert_eq!(kind(&error), "NotJson", "{data:?}");
    }
}
