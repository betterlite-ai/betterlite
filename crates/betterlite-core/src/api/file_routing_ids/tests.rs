#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_openai::batches::records::BatchObject;
use rstest::rstest;

use super::batch::encode_batch_response_ids;
use super::error::FileRoutingIdsError;
use super::legacy::{
    convert_b64_uid_to_unified_uid, decode_legacy_unified_id, is_litellm_executed_batch,
    resolve_output_model, target_model_names, unified_batch_id, unified_batch_model_id,
    validate_file_list_limit,
};
use super::routing::{
    decode_model_from_file_id, encode_file_id_with_model, get_original_file_id,
    is_model_embedded_id,
};
use super::{FileRoutingIds, RoutingIdType, decode_file_routing_ids};
use crate::api::api_boundaries::{ApiLimitError, ApiLimits};
use crate::api::managed_resource_ids::decode_unified_id;

const UNIFIED_COLON: &str = "bGl0ZWxsbV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ";
const UNIFIED_SEMICOLON: &str = "bGl0ZWxsbV9wcm94eTttb2RlbF9pZDptO2xsbV9iYXRjaF9pZDpi";
const UNIFIED_JUNK: &str = "bGl0ZWxsbV9wcm94eWp1bms";
const ENCODED_TARGETS: &str = "bGl0ZWxsbV9wcm94eTp4O3RhcmdldF9tb2RlbF9uYW1lcyxhLCBiLCxh";
const FILE_PAYLOAD: &str = "file-bGl0ZWxsbTpmaWxlLWE7bW9kZWwscHVibGlj";
const BARE_PAYLOAD: &str = "bGl0ZWxsbTpmaWxlLWE7bW9kZWwscHVibGlj";
const BATCH_PAYLOAD: &str = "batch_bGl0ZWxsbTpiYXRjaF9hO21vZGVsLHB1YmxpYw";
const NUMERIC_PAYLOAD: &str = "batch_bGl0ZWxsbToxMjM7bW9kZWwscHVibGlj";
const EMPTY_MODEL: &str = "file-bGl0ZWxsbToxMjM7bW9kZWws";
const REPEATED_MODEL: &str = "file-bGl0ZWxsbTpmaWxlLWE7bW9kZWwsO21vZGVsLHR3bw";
const EMPTY_ORIGINAL: &str = "file-bGl0ZWxsbTo7bW9kZWwseA";
const UNANCHORED: &str = "file-bGl0ZWxsbTpmaWxlLWE7eDttb2RlbCxi";
const NESTED: &str = "file-bGl0ZWxsbTphO2xsbV9iYXRjaF9pZDpsaXRlbGxtOmI7bW9kZWwsYw";

#[rstest]
#[case::absent(None, None)]
#[case::minimum(Some(1), None)]
#[case::maximum(Some(10_000), None)]
#[case::zero(Some(0), Some(("below minimum", ">= 1", "integer_below_min_value")))]
#[case::negative(Some(-1), Some(("below minimum", ">= 1", "integer_below_min_value")))]
#[case::above(Some(10_001), Some(("above maximum", "<= 10000", "integer_above_max_value")))]
fn list_limit_bounds(#[case] limit: Option<i64>, #[case] expected: Option<(&str, &str, &str)>) {
    let outcome = validate_file_list_limit(limit);
    let Some((bound, expected_value, openai_code)) = expected else {
        assert_eq!(outcome, Ok(()));
        return;
    };
    let Err(FileRoutingIdsError::InvalidLimit(exception)) = outcome else {
        panic!("a rejected limit must carry the proxy exception");
    };
    assert_eq!(
        exception.message,
        format!(
            "Invalid 'limit': integer {bound} value. Expected a value {expected_value}, but got {} instead.",
            limit.unwrap()
        )
    );
    assert_eq!(exception.error_type, "invalid_request_error");
    assert_eq!(exception.param.as_deref(), Some("limit"));
    assert_eq!(exception.code, "400");
    assert_eq!(exception.openai_code.as_deref(), Some(openai_code));
    assert!(exception.headers.is_empty());
}

#[test]
fn legacy_prefix_and_targets() {
    assert_eq!(
        decode_legacy_unified_id(UNIFIED_SEMICOLON),
        Some("litellm_proxy;model_id:m;llm_batch_id:b".to_owned())
    );
    // B1016's detector needs the colon, so the same legacy id passes only this file-prefix rule
    assert_eq!(decode_unified_id(UNIFIED_SEMICOLON), None);
    assert_eq!(
        convert_b64_uid_to_unified_uid(UNIFIED_JUNK),
        "litellm_proxyjunk"
    );
    assert_eq!(convert_b64_uid_to_unified_uid("file-1"), "file-1");
    assert_eq!(
        target_model_names("target_model_names,a, b,,a"),
        vec!["a", "b", "", "a"]
    );
    assert_eq!(target_model_names(BARE_PAYLOAD), Vec::<String>::new());
}

#[rstest]
#[case::raw(Some("litellm_proxy:x;target_model_names,a, b,,a"), Some("a,b,,a"))]
#[case::encoded(Some(ENCODED_TARGETS), Some("a,b,,a"))]
#[case::absent(None, Some("fb"))]
#[case::no_targets(Some("file-1"), Some("fb"))]
#[case::empty(Some(""), Some("fb"))]
fn output_model_selection(#[case] file_id: Option<&str>, #[case] expected: Option<&str>) {
    assert_eq!(
        resolve_output_model(file_id, Some("fb")),
        expected.map(str::to_owned)
    );
}

#[test]
fn output_model_selection_without_a_fallback() {
    assert_eq!(resolve_output_model(None, None), None);
    assert_eq!(resolve_output_model(Some("file-1"), None), None);
}

#[rstest]
#[case::colon_prefix(UNIFIED_COLON, Some("litellm_proxy:file;unified_id,u"))]
#[case::no_colon_prefix(UNIFIED_SEMICOLON, Some("litellm_proxy;model_id:m;llm_batch_id:b"))]
#[case::bare_prefix("bGl0ZWxsbV9wcm94eQ", Some("litellm_proxy"))]
#[case::data_after_padding("bGl0ZWxsbV9wcm94eQ==A", Some("litellm_proxy\0"))]
#[case::other_prefix("b3RoZXI6ZmlsZQ", None)]
#[case::raw_id("file-1", None)]
#[case::invalid_utf8("/w==", None)]
#[case::empty("", None)]
#[case::non_ascii("\u{e9}", None)]
#[case::single_symbol("A", None)]
#[case::interior_padding("bGl0ZWxs=bV9wcm94eQ", None)]
fn legacy_encoded_edges(#[case] encoded: &str, #[case] expected: Option<&str>) {
    assert_eq!(
        decode_legacy_unified_id(encoded),
        expected.map(str::to_owned)
    );
    match expected {
        Some(decoded) => assert_eq!(convert_b64_uid_to_unified_uid(encoded), decoded),
        None => assert_eq!(convert_b64_uid_to_unified_uid(encoded), encoded),
    }
}

#[rstest]
#[case::plain("target_model_names,a, b,,a", vec!["a", "b", "", "a"])]
#[case::empty_pieces("target_model_names,,,", vec!["", "", ""])]
#[case::later_field("x;target_model_names,a;target_model_names,b", vec!["a"])]
#[case::empty_then_later("target_model_names,;target_model_names,b", vec!["b"])]
#[case::no_semicolon_boundary("xtarget_model_names,a,b;c", vec!["a", "b"])]
#[case::space_only("target_model_names, ", vec![""])]
#[case::python_space("target_model_names,\u{a0}a\u{3000}, \u{1c}b", vec!["a", "b"])]
#[case::missing("nothing", vec![])]
#[case::empty("", vec![])]
fn target_model_text(#[case] text: &str, #[case] expected: Vec<&str>) {
    let expected_names = expected.into_iter().map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(target_model_names(text), expected_names);
}

#[rstest]
#[case::plain("model_id:m", Some("m"))]
#[case::proxy_form("litellm_proxy;model_id:m;llm_batch_id:b", Some("m"))]
#[case::empty_value("litellm_proxy;model_id:;llm_batch_id:b", Some(""))]
#[case::missing("no marker here", None)]
#[case::first_occurrence("xmodel_id:a;model_id:b", Some("a"))]
fn batch_identifier_fields(#[case] file_id: &str, #[case] expected: Option<&str>) {
    assert_eq!(unified_batch_model_id(file_id), expected.map(str::to_owned));
}

#[rstest]
#[case::semicolon("llm_batch_id:b;other", "b")]
#[case::comma("llm_batch_id:b,other", "b")]
#[case::generic("generic_response_id:g,other", "g")]
#[case::empty("llm_batch_id:", "")]
#[case::first_marker("xllm_batch_id:one;llm_batch_id:two", "one")]
fn batch_id_field_terminators(#[case] file_id: &str, #[case] expected: &str) {
    assert_eq!(unified_batch_id(file_id), Ok(expected.to_owned()));
}

#[rstest]
#[case::neither_marker("neither")]
#[case::marker_without_colon("llm_batch_idX;generic_response_id:g")]
#[case::empty("")]
fn batch_id_requires_a_marker(#[case] file_id: &str) {
    assert_eq!(
        unified_batch_id(file_id),
        Err(FileRoutingIdsError::MissingBatchMarker)
    );
}

#[rstest]
#[case::executed("litellm_proxy;model_id:m;llm_batch_id:litellm_batch_1", true)]
#[case::empty_suffix("llm_batch_id:litellm_batch_", true)]
#[case::embedded_marker("xllm_batch_id:litellm_batch_1", true)]
#[case::plain_batch("llm_batch_id:not_litellm_batch_1", false)]
#[case::generic_response("generic_response_id:litellm_batch_1", false)]
#[case::no_marker("litellm_batch_1", false)]
#[case::empty("", false)]
fn executed_batch_detection(#[case] decoded: &str, #[case] expected: bool) {
    assert_eq!(is_litellm_executed_batch(decoded), expected);
}

#[test]
fn model_embedded_roundtrip() {
    assert_eq!(
        encode_file_id_with_model("file-a", "public", RoutingIdType::File),
        FILE_PAYLOAD
    );
    assert_eq!(
        encode_file_id_with_model("batch_a", "public", RoutingIdType::Batch),
        BATCH_PAYLOAD
    );
    assert_eq!(
        encode_file_id_with_model("123", "public", RoutingIdType::Batch),
        NUMERIC_PAYLOAD
    );
    // a recognized input prefix wins over a conflicting id type
    assert_eq!(
        encode_file_id_with_model("file-a", "public", RoutingIdType::Batch),
        FILE_PAYLOAD
    );
    assert_eq!(
        encode_file_id_with_model("batch_a", "public", RoutingIdType::File),
        BATCH_PAYLOAD
    );
    assert_eq!(
        decode_model_from_file_id(FILE_PAYLOAD),
        Some("public".to_owned())
    );
    assert_eq!(get_original_file_id(FILE_PAYLOAD), "file-a");
    assert!(is_model_embedded_id(FILE_PAYLOAD));
}

#[rstest]
#[case::bare(BARE_PAYLOAD, Some("public"), "file-a")]
#[case::batch_prefix(BATCH_PAYLOAD, Some("public"), "batch_a")]
#[case::numeric(NUMERIC_PAYLOAD, Some("public"), "123")]
#[case::empty_model(EMPTY_MODEL, None, "123")]
#[case::repeated(REPEATED_MODEL, Some("two"), "file-a")]
#[case::empty_original(EMPTY_ORIGINAL, Some("x"), EMPTY_ORIGINAL)]
#[case::unanchored(UNANCHORED, Some("b"), UNANCHORED)]
#[case::nested(NESTED, Some("c"), "b")]
#[case::malformed("file-notbase64!", None, "file-notbase64!")]
#[case::empty("", None, "")]
fn embedded_decoding(#[case] encoded: &str, #[case] model: Option<&str>, #[case] original: &str) {
    assert_eq!(decode_model_from_file_id(encoded), model.map(str::to_owned));
    assert_eq!(get_original_file_id(encoded), original);
}

#[rstest]
#[case::embedded(FILE_PAYLOAD, true)]
#[case::empty_original(EMPTY_ORIGINAL, true)]
#[case::empty_model(EMPTY_MODEL, false)]
#[case::malformed("file-notbase64!", false)]
#[case::empty("", false)]
fn model_embedded_detection(#[case] encoded: &str, #[case] expected: bool) {
    assert_eq!(is_model_embedded_id(encoded), expected);
}

#[rstest]
#[case::stripped(
    "file-bGl0ZWxsbTpmaWxlLWE7bW9kZWwsIHB1YmxpYyA",
    Some("public"),
    "file-a"
)]
#[case::unicode_space("file-bGl0ZWxsbTpmaWxlLWE7bW9kZWwsIG3jgIA", Some("m"), "file-a")]
fn embedded_value_edges(
    #[case] encoded: &str,
    #[case] model: Option<&str>,
    #[case] original: &str,
) {
    assert_eq!(decode_model_from_file_id(encoded), model.map(str::to_owned));
    assert_eq!(get_original_file_id(encoded), original);
}

fn batch_object(source: &str) -> BatchObject {
    serde_json::from_str(source).unwrap()
}

#[test]
fn batch_response_encoding() {
    let mut response = batch_object(
        r#"{"id":"b","completion_window":"24h","created_at":1,"endpoint":"/v1/chat/completions","input_file_id":"i","object":"batch","status":"completed","output_file_id":"o","error_file_id":"e","metadata":{"k":"v"}}"#,
    );
    encode_batch_response_ids(&mut response, "m");
    assert_eq!(response.id, "batch_bGl0ZWxsbTpiO21vZGVsLG0");
    assert_eq!(response.input_file_id, "file-bGl0ZWxsbTppO21vZGVsLG0");
    assert_eq!(
        response.output_file_id.as_deref(),
        Some("file-bGl0ZWxsbTpvO21vZGVsLG0")
    );
    assert_eq!(
        response.error_file_id.as_deref(),
        Some("file-bGl0ZWxsbTplO21vZGVsLG0")
    );
    assert_eq!(
        response
            .metadata
            .as_ref()
            .and_then(|fields| fields.get("k"))
            .map(String::as_str),
        Some("v")
    );
    assert_eq!(response.created_at, 1);
    assert_eq!(response.endpoint, "/v1/chat/completions");
}

#[test]
fn batch_response_encoding_skips_empty_and_absent_ids() {
    let mut empty_id = batch_object(
        r#"{"id":"","completion_window":"24h","created_at":1,"endpoint":"/v1/chat/completions","input_file_id":"i","object":"batch","status":"completed","output_file_id":"o","error_file_id":"e"}"#,
    );
    encode_batch_response_ids(&mut empty_id, "m");
    assert_eq!(empty_id.id, "");
    assert_eq!(empty_id.input_file_id, "i");
    assert_eq!(empty_id.output_file_id.as_deref(), Some("o"));
    assert_eq!(empty_id.error_file_id.as_deref(), Some("e"));
    let mut absent = batch_object(
        r#"{"id":"b","completion_window":"24h","created_at":1,"endpoint":"/v1/chat/completions","input_file_id":"","object":"batch","status":"completed"}"#,
    );
    encode_batch_response_ids(&mut absent, "m");
    assert_eq!(absent.id, "batch_bGl0ZWxsbTpiO21vZGVsLG0");
    assert_eq!(absent.input_file_id, "");
    assert_eq!(absent.output_file_id, None);
    assert_eq!(absent.error_file_id, None);
    let mut empty_secondary = batch_object(
        r#"{"id":"b","completion_window":"24h","created_at":1,"endpoint":"/v1/chat/completions","input_file_id":"i","object":"batch","status":"completed","output_file_id":"","error_file_id":""}"#,
    );
    encode_batch_response_ids(&mut empty_secondary, "m");
    assert_eq!(empty_secondary.output_file_id.as_deref(), Some(""));
    assert_eq!(empty_secondary.error_file_id.as_deref(), Some(""));
}

#[test]
fn a_second_encoding_wraps_again() {
    let mut response = batch_object(
        r#"{"id":"b","completion_window":"24h","created_at":1,"endpoint":"/v1/chat/completions","input_file_id":"i","object":"batch","status":"completed","output_file_id":"o","error_file_id":"e"}"#,
    );
    encode_batch_response_ids(&mut response, "m");
    encode_batch_response_ids(&mut response, "m");
    assert_eq!(
        response.id,
        "batch_bGl0ZWxsbTpiYXRjaF9iR2wwWld4c2JUcGlPMjF2WkdWc0xHMDttb2RlbCxt"
    );
    assert_eq!(
        decode_model_from_file_id(&response.id),
        Some("m".to_owned())
    );
}

#[test]
fn boundary_reports_every_read() {
    let ids = decode_file_routing_ids(UNIFIED_COLON.as_bytes(), ApiLimits::default()).unwrap();
    assert_eq!(
        ids.unified_id.as_deref(),
        Some("litellm_proxy:file;unified_id,u")
    );
    assert_eq!(ids.model, None);
    assert_eq!(ids.original_id, UNIFIED_COLON);
    let routing = decode_file_routing_ids(FILE_PAYLOAD.as_bytes(), ApiLimits::default()).unwrap();
    assert_eq!(
        routing,
        FileRoutingIds {
            unified_id: None,
            model: Some("public".to_owned()),
            original_id: "file-a".to_owned(),
        }
    );
    let raw = decode_file_routing_ids(b"file-1", ApiLimits::default()).unwrap();
    assert_eq!(
        raw,
        FileRoutingIds {
            unified_id: None,
            model: None,
            original_id: "file-1".to_owned(),
        }
    );
}

#[test]
fn boundary_enforces_limits_and_utf8() {
    let limits = ApiLimits {
        max_json_bytes: 8,
        max_event_bytes: usize::MAX,
        max_items: usize::MAX,
    };
    drop(decode_file_routing_ids(b"12345678", limits).unwrap());
    assert_eq!(
        decode_file_routing_ids(b"123456789", limits),
        Err(FileRoutingIdsError::Limit(ApiLimitError::JsonTooLarge {
            observed: 9,
            limit: 8
        }))
    );
    assert!(matches!(
        decode_file_routing_ids(&[0xff], ApiLimits::default()),
        Err(FileRoutingIdsError::NotUtf8 { .. })
    ));
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/file_routing_ids");
    let replayed = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .map(|bytes| drop(decode_file_routing_ids(&bytes, ApiLimits::default())))
        .count();
    assert!(replayed >= 10);
}
