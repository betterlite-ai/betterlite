#![cfg(test)]
#![expect(
    clippy::disallowed_methods,
    reason = "insta uses Into internally to normalize its snapshot name"
)]

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;

use flate2::Compression;
use flate2::write::GzEncoder;
use indexmap::IndexMap;
use rstest::rstest;

use super::{
    DecodedEvent, DecodedSpan, NormalizedSpan, OTLP_MAX_BODY_BYTES, SpanCodec,
    decode_content_encoding, decode_otlp, span_row, truncate,
};
use crate::otlp_errors::CodecError;
use crate::trace_types::SpanType;

const TOO_LARGE: &str = "OTLP body exceeds 16777216 bytes";
const INVALID_GZIP: &str = "Invalid OTLP gzip body";
const UNSUPPORTED: &str = "Unsupported OTLP content encoding";

struct JsonCodec;

impl SpanCodec for JsonCodec {
    fn decode(
        &self,
        payload: &[u8],
        content_type: Option<&str>,
    ) -> Result<Vec<DecodedSpan>, CodecError> {
        if content_type == Some("overflow") {
            return Err(CodecError::Overflow("too many spans".to_owned()));
        }
        serde_json::from_slice(payload).map_err(|error| CodecError::Malformed(error.to_string()))
    }
}

fn gzip(members: &[&[u8]]) -> Vec<u8> {
    let mut body = Vec::new();
    for member in members {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(member).unwrap();
        body.extend(encoder.finish().unwrap());
    }
    body
}

fn attributes(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|&(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

fn span() -> DecodedSpan {
    DecodedSpan {
        trace_id: "0af7651916cd43dd8448eb211c80319c".to_owned(),
        span_id: "b7ad6b7169203331".to_owned(),
        parent_span_id: "00f067aa0ba902b7".to_owned(),
        trace_state: "vendor=1".to_owned(),
        name: "chat gpt-4o".to_owned(),
        kind: "SPAN_KIND_CLIENT".to_owned(),
        resource_attributes: attributes(&[("service.name", "agent"), ("host", "a1")]),
        scope_name: "openinference".to_owned(),
        scope_version: "1.2".to_owned(),
        attributes: attributes(&[
            ("gen_ai.request.model", "gpt-4o"),
            ("llm.provider", "openai"),
            ("big", &"v".repeat(0x1_0001)),
        ]),
        start_ns: 1_000,
        end_ns: 4_500,
        status_code: "STATUS_CODE_ERROR".to_owned(),
        status_message: String::new(),
        events: vec![DecodedEvent {
            name: "exception".to_owned(),
            attributes: attributes(&[
                ("exception.type", "ValueError"),
                ("exception.message", "bad input"),
            ]),
        }],
        normalized: NormalizedSpan {
            observation_type: SpanType::Llm,
            agent_name: "planner".to_owned(),
            litellm_request_id: "chatcmpl-1".to_owned(),
            model: "gpt-4o".to_owned(),
            input_tokens: 12,
            output_tokens: 3,
            input: r#"[{"role": "user", "content": "hi"}]"#.to_owned(),
            output: "o".repeat(0x1_0002),
        },
        consumed_attributes: vec!["gen_ai.request.model".to_owned()],
    }
}

#[rstest]
#[case::no_encoding(b"abc".to_vec(), None, Ok(b"abc".to_vec()))]
#[case::identity(b"abc".to_vec(), Some("identity"), Ok(b"abc".to_vec()))]
#[case::identity_any_case(b"abc".to_vec(), Some("IDENTITY"), Ok(b"abc".to_vec()))]
#[case::brotli(b"abc".to_vec(), Some("br"), Err(UNSUPPORTED))]
#[case::empty_encoding(b"abc".to_vec(), Some(""), Err(UNSUPPORTED))]
#[case::gzip(gzip(&[b"hello"]), Some("gzip"), Ok(b"hello".to_vec()))]
#[case::gzip_any_case(gzip(&[b"hello"]), Some("GZip"), Ok(b"hello".to_vec()))]
#[case::gzip_members(gzip(&[b"a", b"b"]), Some("gzip"), Ok(b"ab".to_vec()))]
#[case::gzip_zero_padding([gzip(&[b"a"]), vec![0; 4]].concat(), Some("gzip"), Ok(b"a".to_vec()))]
#[case::gzip_trailing_bytes([gzip(&[b"a"]), b"zz".to_vec()].concat(), Some("gzip"), Err(INVALID_GZIP))]
#[case::gzip_cut_short(gzip(&[b"hello"])[..12].to_vec(), Some("gzip"), Err(INVALID_GZIP))]
#[case::gzip_empty(Vec::new(), Some("gzip"), Ok(Vec::new()))]
#[case::not_gzip(b"not gzip".to_vec(), Some("gzip"), Err(INVALID_GZIP))]
#[case::body_at_limit(vec![0; OTLP_MAX_BODY_BYTES], None, Ok(vec![0; OTLP_MAX_BODY_BYTES]))]
#[case::body_over_limit(vec![0; OTLP_MAX_BODY_BYTES + 1], None, Err(TOO_LARGE))]
#[case::gzip_body_over_limit(vec![0; OTLP_MAX_BODY_BYTES + 1], Some("gzip"), Err(TOO_LARGE))]
#[case::inflated_to_limit(gzip(&[&vec![0; OTLP_MAX_BODY_BYTES]]), Some("gzip"), Ok(vec![0; OTLP_MAX_BODY_BYTES]))]
#[case::inflated_over_limit(gzip(&[&vec![0; OTLP_MAX_BODY_BYTES], b"x"]), Some("gzip"), Err(TOO_LARGE))]
fn decodes_content_encoding_like_litellm(
    #[case] body: Vec<u8>,
    #[case] encoding: Option<&str>,
    #[case] expected: Result<Vec<u8>, &str>,
) {
    let decoded = decode_content_encoding(&body, encoding);
    assert_eq!(
        decoded
            .map(|payload| payload.to_vec())
            .map_err(|error| error.to_string()),
        expected.map_err(str::to_owned)
    );
}

#[test]
fn snapshots_a_decoded_span_row() {
    let body = serde_json::to_vec(&[span()]).unwrap();
    let rows = decode_otlp(
        &JsonCodec,
        &gzip(&[&body]),
        Some("application/json"),
        Some("gzip"),
    )
    .unwrap();
    let [mut row] = <[_; 1]>::try_from(rows).unwrap();
    // The truncated values are 64 KiB long, so the snapshot keeps their length and tail
    let big = &row.span_attributes["big"];
    row.span_attributes["big"] = format!("{} {}", big.len(), big.get(0xfff0..).unwrap());
    row.output = format!("{} {}", row.output.len(), row.output.get(0xfff0..).unwrap());
    insta::assert_debug_snapshot!("decoded_span_row", row);
}

#[rstest]
#[case::malformed(b"{".to_vec(), None, Some("gzip"), INVALID_GZIP)]
#[case::unsupported(b"[]".to_vec(), None, Some("deflate"), UNSUPPORTED)]
#[case::codec_overflow(b"[]".to_vec(), Some("overflow"), None, "too many spans")]
#[case::codec_malformed(b"[".to_vec(), None, None, "EOF while parsing a list at line 1 column 1")]
fn reports_decode_failures(
    #[case] body: Vec<u8>,
    #[case] content_type: Option<&str>,
    #[case] encoding: Option<&str>,
    #[case] message: &str,
) {
    let error = decode_otlp(&JsonCodec, &body, content_type, encoding).unwrap_err();
    assert_eq!(error.to_string(), message);
}

#[rstest]
#[case::status_message("failed", &[("exception.message", "bad")], "failed")]
#[case::exception_message("", &[("exception.type", "ValueError"), ("exception.message", "bad")], "bad")]
#[case::empty_exception_message("", &[("exception.type", "ValueError"), ("exception.message", "")], "ValueError")]
#[case::exception_type("", &[("exception.type", "ValueError")], "ValueError")]
#[case::bare_exception("", &[], "")]
fn reads_the_status_message(
    #[case] status: &str,
    #[case] exception: &[(&str, &str)],
    #[case] expected: &str,
) {
    let mut decoded = span();
    decoded.status_message = status.to_owned();
    decoded.events = vec![
        DecodedEvent {
            name: "log".to_owned(),
            attributes: attributes(&[("exception.message", "other")]),
        },
        DecodedEvent {
            name: "exception".to_owned(),
            attributes: attributes(exception),
        },
        DecodedEvent {
            name: "exception".to_owned(),
            attributes: attributes(&[("exception.message", "later")]),
        },
    ];
    assert_eq!(span_row(&decoded).status_message, expected);
}

#[test]
fn status_message_is_empty_without_an_exception() {
    let mut decoded = span();
    decoded.events.clear();
    assert_eq!(span_row(&decoded).status_message, "");
}

#[rstest]
#[case::response_id(&[("gen_ai.response.id", "resp-9")], "resp-9")]
#[case::empty_response_id(&[("gen_ai.response.id", "")], "chatcmpl-1")]
#[case::no_response_id(&[], "chatcmpl-1")]
fn prefers_the_response_id(#[case] pairs: &[(&str, &str)], #[case] expected: &str) {
    let mut decoded = span();
    decoded.attributes = attributes(pairs);
    assert_eq!(span_row(&decoded).litellm_request_id, expected);
}

#[test]
fn service_name_defaults_to_empty() {
    let mut decoded = span();
    decoded.resource_attributes.clear();
    assert_eq!(span_row(&decoded).service_name, "");
}

#[rstest]
#[case::short("abc", "abc")]
#[case::at_limit(&"x".repeat(0x1_0000), &"x".repeat(0x1_0000))]
#[case::over_limit(&"x".repeat(0x1_0001), &format!("{}\u{2026}[truncated 1 bytes]", "x".repeat(0x1_0000)))]
#[case::split_character(&format!("{}\u{e9}", "x".repeat(0xffff)), &format!("{}\u{2026}[truncated 2 bytes]", "x".repeat(0xffff)))]
fn truncates_attribute_values(#[case] value: &str, #[case] expected: &str) {
    assert_eq!(truncate(value), expected);
}

#[rstest]
fn replays_the_otlp_decode_corpus(#[files("fuzz/corpus/otlp_decode/*")] path: PathBuf) {
    let body = fs::read(path).unwrap();
    drop(decode_otlp(
        &JsonCodec,
        &body,
        Some("application/json"),
        None,
    ));
    drop(decode_otlp(
        &JsonCodec,
        &body,
        Some("application/json"),
        Some("gzip"),
    ));
}
