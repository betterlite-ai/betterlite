#![cfg(test)]

use std::collections::BTreeMap;
use std::path::Path;
use std::{env, fs, process, slice};

use rstest::rstest;
use serde_json::{Map, Value};

use crate::case::{
    Compare, Request, RequestBody, UpstreamAnswer, decode_http_body, decode_http_case,
};
use crate::compare::compare;
use crate::compare::multipart::first_part_difference;
use crate::http::{Response, client, parse_body, send};
use crate::mock::{MockProvider, ProviderRequest};
use crate::multipart_oracle::Part;
use crate::multipart_oracle::capture::{decode_multipart_capture, split_capture_input};
use crate::multipart_oracle::request::read_fixture;
use crate::routing_mock_scripts::MockScript;

const BINARY: &[u8] = b"\x00\xffbinary\x80\n";
const LIMIT: usize = 16 * 1024 * 1024;

fn decode(body_kind: &str, body: &str) -> Result<RequestBody, String> {
    let yaml = format!(
        "{{method: POST, path: /upload, headers: {{}}, body_kind: {body_kind}, body: {body}}}"
    );
    decode_http_body(yaml.as_bytes()).map_err(|error| error.to_string())
}

fn multipart(parts: &str) -> (String, Vec<u8>) {
    match decode("multipart", &format!("{{parts: [{parts}]}}")).unwrap() {
        RequestBody::Raw {
            content_type,
            bytes,
        } => (content_type, bytes),
        RequestBody::Empty | RequestBody::Json(_) => panic!("multipart body expected"),
    }
}

fn part(name: &str, filename: Option<&str>, content_type: Option<&str>, body: &[u8]) -> Part {
    Part {
        name: name.to_owned(),
        filename: filename.map(str::to_owned),
        content_type: content_type.map(str::to_owned),
        body: body.to_vec(),
    }
}

fn wire(bytes: &[u8]) -> Value {
    Value::Array(bytes.iter().map(|byte| Value::from(*byte)).collect())
}

fn provider(content_type: &str, bytes: &[u8]) -> ProviderRequest {
    ProviderRequest {
        method: "POST".to_owned(),
        path: "/upload".to_owned(),
        headers: BTreeMap::from([
            ("content-type".to_owned(), content_type.to_owned()),
            ("x-test".to_owned(), "one".to_owned()),
        ]),
        body: wire(bytes),
    }
}

fn upstream_pointer(left: &ProviderRequest, right: &ProviderRequest) -> Option<String> {
    let response = Response {
        status: 200,
        headers: BTreeMap::new(),
        body: Value::Null,
    };
    let headers = Compare {
        headers: vec![],
        headers_present: vec![],
        upstream_headers: vec!["content-type".to_owned(), "x-test".to_owned()],
        ignore: vec![],
    };
    compare(
        &response,
        &response,
        slice::from_ref(left),
        slice::from_ref(right),
        &headers,
    )
    .err()
    .map(|difference| difference.pointer)
}

fn rebound(content_type: &str, bytes: &[u8]) -> (String, Vec<u8>) {
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    (
        content_type.replace("betterlite-oracle-0", "other"),
        text.replace("betterlite-oracle-0", "other").into_bytes(),
    )
}

#[test]
fn preserves_legacy_json_default() {
    let yaml = "{method: GET, path: /, headers: {}, body: {multipart: value, bytes: value}}";
    let expected = br#"{"bytes":"value","multipart":"value"}"#.to_vec();
    assert_eq!(
        decode_http_body(yaml.as_bytes()).unwrap(),
        RequestBody::Json(expected)
    );
    let empty = "{method: GET, path: /, headers: {}, body: null}";
    assert_eq!(
        decode_http_body(empty.as_bytes()).unwrap(),
        RequestBody::Empty
    );
}

#[test]
fn json_body_collision() {
    let expected = br#"{"bytes":"value","json":"value","multipart":"value"}"#.to_vec();
    assert_eq!(
        decode("json", "{multipart: value, bytes: value, json: value}"),
        Ok(RequestBody::Json(expected))
    );
    assert_eq!(
        decode("multipart", "{multipart: value}"),
        Err("field \"multipart\" is not allowed".to_owned())
    );
    assert_eq!(
        decode("bytes", "{bytes: value}"),
        Err("field \"bytes\" is not allowed".to_owned())
    );
}

#[test]
fn ordered_duplicate_parts() {
    let (content_type, bytes) = multipart(
        "{name: file, filename: a.txt, content_type: text/plain, text: one}, \
         {name: file, filename: a.txt, content_type: text/plain, fixture: binary-response.bin}, \
         {name: field, text: value}",
    );
    assert_eq!(
        decode_multipart_capture(&content_type, &bytes).unwrap(),
        vec![
            part("file", Some("a.txt"), Some("text/plain"), b"one"),
            part("file", Some("a.txt"), Some("text/plain"), BINARY),
            part("field", None, None, b"value"),
        ]
    );
}

#[test]
fn rejects_dual_or_missing_payload() {
    let message = Err("part must contain exactly one payload".to_owned());
    assert_eq!(
        decode(
            "multipart",
            "{parts: [{name: file, text: x, fixture: payload.bin}]}"
        ),
        message
    );
    assert_eq!(decode("multipart", "{parts: [{name: file}]}"), message);
}

#[test]
fn rejects_empty_part_name() {
    let message = Err("part name must not be empty".to_owned());
    assert_eq!(decode("multipart", "{parts: [{text: x}]}"), message);
    assert_eq!(
        decode("multipart", "{parts: [{name: '', text: x}]}"),
        message
    );
}

#[test]
fn sends_real_wire_bytes() {
    let answer = UpstreamAnswer {
        status: 200,
        headers: BTreeMap::new(),
        body: Some(Value::Object(Map::new())),
        events: None,
    };
    let mock =
        MockProvider::start(MockScript::try_from(&[answer.clone(), answer][..]).unwrap()).unwrap();
    let (content_type, bytes) =
        multipart("{name: file, filename: a.bin, fixture: binary-response.bin}");
    let multipart_request: Request = serde_saphyr::from_str(
        "{method: POST, path: /upload, headers: {}, body_kind: multipart, body: {parts: [{name: file, filename: a.bin, fixture: binary-response.bin}]}}",
    )
    .unwrap();
    send(&client().unwrap(), &mock.base_url, &multipart_request).unwrap();
    let bytes_request: Request = serde_saphyr::from_str(
        "{method: POST, path: /upload, headers: {}, body_kind: bytes, body: {fixture: binary-response.bin, content_type: application/octet-stream}}",
    )
    .unwrap();
    send(&client().unwrap(), &mock.base_url, &bytes_request).unwrap();
    let calls = mock.finish().unwrap();
    assert_eq!(
        calls[0].request.headers["content-type"],
        "multipart/form-data; boundary=betterlite-oracle-0"
    );
    assert_eq!(calls[0].request.headers["content-type"], content_type);
    assert_eq!(calls[0].request.body, wire(&bytes));
    assert_eq!(
        calls[1].request.headers["content-type"],
        "application/octet-stream"
    );
    assert_eq!(calls[1].request.body, wire(BINARY));
    assert_eq!(calls[1].request.path, "/upload");
}

#[test]
fn binary_answer_exact() {
    let answer = UpstreamAnswer {
        status: 200,
        headers: BTreeMap::from([(
            "content-type".to_owned(),
            "application/octet-stream".to_owned(),
        )]),
        body: Some(
            serde_json::from_str(r#"{"body_kind": "bytes", "fixture": "binary-response.bin", "content_type": "application/octet-stream"}"#).unwrap(),
        ),
        events: None,
    };
    let mock = MockProvider::start(MockScript::try_from(&[answer][..]).unwrap()).unwrap();
    let request: Request =
        serde_saphyr::from_str("{method: GET, path: /file, headers: {}, body: null}").unwrap();
    let left = send(&client().unwrap(), &mock.base_url, &request).unwrap();
    assert_eq!(left.status, 200);
    assert_eq!(left.headers["content-type"], "application/octet-stream");
    assert_eq!(left.body, wire(BINARY));
    let mut right = left.clone();
    right.body = wire(b"\x00\xfebinary\x80\n");
    let difference = compare(&left, &right, &[], &[], &Compare::default()).unwrap_err();
    assert_eq!(difference.pointer, "/response/bytes");
    let mut one_sided = left.clone();
    one_sided
        .headers
        .insert("content-type".to_owned(), "text/plain".to_owned());
    one_sided.body = wire(b"\x00\xfebinary\x80\n");
    let one_sided_difference =
        compare(&left, &one_sided, &[], &[], &Compare::default()).unwrap_err();
    assert_eq!(one_sided_difference.pointer, "/1");
}

#[test]
fn rejects_text_binary_mismatch() {
    let text = BTreeMap::from([("content-type".to_owned(), "text/plain".to_owned())]);
    let message = parse_body(&text, BINARY).unwrap_err().to_string();
    assert!(
        message.starts_with("response body is not valid UTF-8"),
        "{message}"
    );
    let binary = BTreeMap::from([(
        "content-type".to_owned(),
        "application/octet-stream".to_owned(),
    )]);
    assert_eq!(parse_body(&binary, BINARY).unwrap(), wire(BINARY));
    let large = UpstreamAnswer {
        status: 200,
        headers: BTreeMap::new(),
        body: Some(Value::String("x".repeat(LIMIT))),
        events: None,
    };
    let mock = MockProvider::start(MockScript::try_from(&[large][..]).unwrap()).unwrap();
    let request: Request =
        serde_saphyr::from_str("{method: GET, path: /, headers: {}, body: null}").unwrap();
    let error = send(&client().unwrap(), &mock.base_url, &request).unwrap_err();
    assert_eq!(error.to_string(), "HTTP response exceeded 16 MiB");
}

#[rstest]
#[case::parent_relative("../outside.bin")]
#[case::absolute_tmp("/tmp/outside.bin")]
#[case::nested_parent("nested/../../outside.bin")]
#[case::absolute_etc("/etc/passwd")]
fn fixture_root_escape_rejected(#[case] name: &str) {
    let body = format!("{{fixture: '{name}', content_type: application/octet-stream}}");
    assert_eq!(
        decode("bytes", &body),
        Err("fixture path escapes multipart fixture root".to_owned())
    );
    let part = format!("{{parts: [{{name: file, fixture: '{name}'}}]}}");
    assert_eq!(
        decode("multipart", &part),
        Err("fixture path escapes multipart fixture root".to_owned())
    );
}

#[test]
fn rejects_unknown_field() {
    assert_eq!(
        decode("multipart", "{parts: [{name: file, text: x, bogus: true}]}"),
        Err("field \"bogus\" is not allowed".to_owned())
    );
    let case = "name: upload\nconfig: |\n  model_list: []\nenv: {}\nrequest: {method: POST, path: /upload, headers: {}, body_kind: multipart, body: {parts: [{name: file, text: x, bogus: true}]}}\nupstream: []\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\n";
    assert_eq!(
        decode_http_case(case.as_bytes()).unwrap_err().to_string(),
        "field \"bogus\" is not allowed"
    );
}

#[test]
fn rejects_duplicate_field() {
    let message = decode("multipart", "{parts: [{name: a, name: b, text: x}]}").unwrap_err();
    assert!(message.contains("duplicate mapping key: name"), "{message}");
}

#[test]
fn rejects_limits() {
    let path = env::temp_dir().join(format!("betterlite-b900-{}.bin", process::id()));
    fs::write(&path, vec![0; LIMIT]).unwrap();
    assert_eq!(read_fixture(&path).unwrap().len(), LIMIT);
    fs::write(&path, vec![0; LIMIT + 1]).unwrap();
    assert_eq!(
        read_fixture(&path).unwrap_err().to_string(),
        "fixture exceeds 16777216 bytes"
    );
    fs::remove_file(&path).unwrap();
    let parts = |count: usize| vec!["{name: x, text: x}"; count].join(", ");
    let (content_type, bytes) = multipart(&parts(128));
    assert_eq!(
        decode_multipart_capture(&content_type, &bytes)
            .unwrap()
            .len(),
        128
    );
    let message = "multipart contains more than 128 parts";
    assert_eq!(
        decode("multipart", &format!("{{parts: [{}]}}", parts(129))),
        Err(message.to_owned())
    );
    let section = b"--a\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\nx\r\n";
    let accepted = [section.repeat(128), b"--a--\r\n".to_vec()].concat();
    assert_eq!(
        decode_multipart_capture("multipart/form-data; boundary=a", &accepted)
            .unwrap()
            .len(),
        128
    );
    let capture = [section.repeat(129), b"--a--\r\n".to_vec()].concat();
    let decoded = decode_multipart_capture("multipart/form-data; boundary=a", &capture);
    assert_eq!(decoded.unwrap_err().to_string(), message);
}

#[test]
fn rejects_captured_body_limits() {
    let head = b"--a\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\n";
    let tail = b"\r\n--a--\r\n";
    let exact = [
        head.to_vec(),
        vec![b'x'; LIMIT - head.len() - tail.len()],
        tail.to_vec(),
    ]
    .concat();
    assert_eq!(
        decode_multipart_capture("multipart/form-data; boundary=a", &exact).unwrap()[0]
            .body
            .len(),
        LIMIT - head.len() - tail.len()
    );
    let over = [
        head.to_vec(),
        vec![b'x'; LIMIT + 1 - head.len() - tail.len()],
        tail.to_vec(),
    ]
    .concat();
    let error = decode_multipart_capture("multipart/form-data; boundary=a", &over).unwrap_err();
    assert_eq!(error.to_string(), "captured body exceeds 16777216 bytes");
}

#[test]
fn rejects_captured_body_limits_over_http() {
    let answer = UpstreamAnswer {
        status: 200,
        headers: BTreeMap::new(),
        body: Some(Value::Object(Map::new())),
        events: None,
    };
    let exact_mock =
        MockProvider::start(MockScript::try_from(&[answer.clone()][..]).unwrap()).unwrap();
    let mut exact_request: Request =
        serde_saphyr::from_str("{method: POST, path: /, headers: {}, body: null}").unwrap();
    exact_request.body = Value::String("x".repeat(LIMIT - 2));
    send(&client().unwrap(), &exact_mock.base_url, &exact_request).unwrap();
    assert_eq!(exact_mock.finish().unwrap().len(), 1);
    let mock = MockProvider::start(MockScript::try_from(&[answer][..]).unwrap()).unwrap();
    let mut request: Request =
        serde_saphyr::from_str("{method: POST, path: /, headers: {}, body: null}").unwrap();
    request.body = Value::String("x".repeat(LIMIT - 1));
    send(&client().unwrap(), &mock.base_url, &request).unwrap();
    assert_eq!(
        mock.finish().unwrap_err().to_string(),
        "mock provider failed: captured body exceeds 16777216 bytes"
    );
}

#[rstest]
#[case::missing_boundary("multipart/form-data")]
#[case::empty_boundary("multipart/form-data; boundary=")]
#[case::unterminated_quote("multipart/form-data; boundary=\"boundary-a")]
#[case::mismatched_boundary("multipart/form-data; boundary=boundary-b")]
#[case::wrong_content_type("text/plain; boundary=boundary-a")]
fn rejects_invalid_boundary(#[case] content_type: &str) {
    let body = b"--boundary-a\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\nx--boundary-a y--boundary-a--\r\n--boundary-a--\r\n";
    let quoted =
        decode_multipart_capture("multipart/form-data; boundary=\"boundary-a\"", body).unwrap();
    assert_eq!(
        quoted,
        vec![part("x", None, None, b"x--boundary-a y--boundary-a--")]
    );
    let delimiter_in_payload = b"--boundary-a\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\nx\r\n--boundary-a y\r\n--boundary-a--\r\n";
    let delimiter_error = decode_multipart_capture(
        "multipart/form-data; boundary=boundary-a",
        delimiter_in_payload,
    )
    .unwrap_err();
    assert_eq!(
        delimiter_error.to_string(),
        "multipart capture has no valid boundary"
    );
    assert_eq!(
        decode_multipart_capture("multipart/form-data; boundary=boundary-a", body).unwrap(),
        quoted
    );
    let error = decode_multipart_capture(content_type, body).unwrap_err();
    assert_eq!(error.to_string(), "multipart capture has no valid boundary");
}

#[test]
fn detects_changed_peer() {
    let (content_type, bytes) =
        multipart("{name: file, content_type: application/octet-stream, text: a}");
    let left = provider(&content_type, &bytes);
    let (other_type, other_bytes) = rebound(&content_type, &bytes);
    assert_eq!(
        upstream_pointer(&left, &provider(&other_type, &other_bytes)),
        None
    );
    let (changed_type, changed_bytes) =
        multipart("{name: file, content_type: application/octet-stream, text: b}");
    assert_eq!(
        upstream_pointer(&left, &provider(&changed_type, &changed_bytes)).as_deref(),
        Some("/upstream/0/parts/0/body")
    );
    let (plain_type, plain_bytes) = multipart("{name: file, content_type: text/plain, text: a}");
    assert_eq!(
        upstream_pointer(&left, &provider(&plain_type, &plain_bytes)).as_deref(),
        Some("/upstream/0/parts/0/content_type")
    );
    let mut header = provider(&other_type, &other_bytes);
    header.headers.insert("x-test".to_owned(), "two".to_owned());
    assert_eq!(
        upstream_pointer(&left, &header).as_deref(),
        Some("/upstream/0/headers/x-test")
    );
    let charset = provider(&format!("{other_type}; charset=utf-8"), &other_bytes);
    assert_eq!(
        upstream_pointer(&left, &charset).as_deref(),
        Some("/upstream/0/headers/content-type")
    );
}

#[test]
fn multipart_diff_pointer() {
    let (content_type, left) =
        multipart("{name: a, text: x}, {name: b, filename: one, text: x}, {name: c, text: x}");
    let (_, right) =
        multipart("{name: a, text: x}, {name: b, filename: two, text: q}, {name: c, text: z}");
    let left_parts = decode_multipart_capture(&content_type, &left).unwrap();
    let right_parts = decode_multipart_capture(&content_type, &right).unwrap();
    let client_pointer =
        first_part_difference(&left_parts, &right_parts).map(|part| format!("/request{part}"));
    assert_eq!(client_pointer.as_deref(), Some("/request/parts/1/filename"));
    let (_, renamed) =
        multipart("{name: a, text: x}, {name: d, filename: one, text: x}, {name: c, text: x}");
    let renamed_pointer = upstream_pointer(
        &provider(&content_type, &left),
        &provider(&content_type, &renamed),
    );
    assert_eq!(renamed_pointer.as_deref(), Some("/upstream/0/parts/1/name"));
    let (_, longer) = multipart(
        "{name: a, text: x}, {name: b, filename: one, text: x}, {name: c, text: x}, {name: e, text: x}",
    );
    let longer_pointer = upstream_pointer(
        &provider(&content_type, &left),
        &provider(&content_type, &longer),
    );
    assert_eq!(longer_pointer.as_deref(), Some("/upstream/0/parts/3"));
}

#[test]
fn preserves_optional_fields() {
    let (content_type, bytes) =
        multipart("{name: x, text: a}, {name: x, filename: '', content_type: '', text: a}");
    let parts = decode_multipart_capture(&content_type, &bytes).unwrap();
    assert_eq!(
        parts,
        vec![
            part("x", None, None, b"a"),
            part("x", Some(""), Some(""), b"a")
        ]
    );
    assert_eq!(
        first_part_difference(&parts[..1], &parts[1..]).as_deref(),
        Some("/parts/0/filename")
    );
}

fn replay_folder(corpus: &Path, folder: &str) -> usize {
    fs::read_dir(corpus.join(folder))
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            let failed = if folder == "multipart_oracle" {
                decode_http_body(&bytes).is_err()
            } else {
                split_capture_input(&bytes)
                    .and_then(|(content_type, body)| decode_multipart_capture(content_type, body))
                    .is_err()
            };
            let rejected = path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("reject-");
            assert_eq!(failed, rejected, "{}", path.display());
        })
        .count()
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus");
    let replayed =
        replay_folder(&corpus, "multipart_oracle") + replay_folder(&corpus, "multipart_capture");
    assert!(replayed >= 20_usize, "{replayed}");
}
