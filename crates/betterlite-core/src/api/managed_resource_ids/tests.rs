#![cfg(test)]

use std::fs;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use rstest::rstest;

use super::error::ManagedResourceIdsError;
use super::{
    ManagedResourceIds, PassthroughManagedIdScope, UnifiedIdParts, decode_managed_resource_ids,
    decode_unified_id, is_base64_encoded_unified_id, model_id, parse_unified_id,
    passthrough_provider_scope, provider_resource_id, resource_type, target_model_names,
    unified_uuid,
};
use crate::api::api_boundaries::{ApiLimitError, ApiLimits};

const FILE_ID: &str = "litellm_proxy:file;unified_id,u";
const VECTOR_STORE_ID: &str = "litellm_proxy:vector_store;unified_id,u;target_model_names,a, b,,a;resource_id,vs_1;model_id,m";

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[rstest]
#[case::empty("", None)]
#[case::upper_openai("OPENAI", Some(PassthroughManagedIdScope::Openai))]
#[case::enum_value("openai", Some(PassthroughManagedIdScope::Openai))]
#[case::upper_azure_ai("AZURE_AI", Some(PassthroughManagedIdScope::Azure))]
#[case::azure("azure", Some(PassthroughManagedIdScope::Azure))]
#[case::prefixed_azure("x.azure", Some(PassthroughManagedIdScope::Azure))]
#[case::prefixed_azure_ai("x.azure_ai", Some(PassthroughManagedIdScope::Azure))]
#[case::prefixed_openai("x.openai", Some(PassthroughManagedIdScope::Openai))]
#[case::compatible("openai_compatible", None)]
#[case::azure_suffix_only("azure.x", None)]
#[case::unprefixed_dot("xazure", None)]
fn provider_namespace(#[case] provider: &str, #[case] scope: Option<PassthroughManagedIdScope>) {
    assert_eq!(passthrough_provider_scope(provider), scope);
}

#[rstest]
#[case::padded(URL_SAFE.encode(FILE_ID))]
#[case::unpadded(URL_SAFE_NO_PAD.encode(FILE_ID))]
#[case::standard_alphabet(STANDARD.encode("litellm_proxy:file;unified_id,???>>>"))]
#[case::url_safe_alphabet(URL_SAFE.encode("litellm_proxy:file;unified_id,???>>>"))]
#[case::space(" bGl0ZWxs bV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ==".to_owned())]
#[case::bang("bGl0ZWxs!bV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ==".to_owned())]
#[case::newline("bGl0\nZWxsbV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ==".to_owned())]
#[case::junk_after_padding("bGl0ZWxsbV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ==!".to_owned())]
#[case::space_before_padding("bGl0ZWxsbV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ ==".to_owned())]
#[case::extra_pads("bGl0ZWxsbV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ====".to_owned())]
#[case::interior_pad("bGl0=ZWxsbV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ==".to_owned())]
#[case::single_trailing_pad("bGl0ZWxsbV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ=".to_owned())]
fn encoded_prefix_detection(#[case] encoded: String) {
    let decoded = is_base64_encoded_unified_id(&encoded, "litellm_proxy:");
    assert!(decoded.is_some_and(|found| found.starts_with("litellm_proxy:file;unified_id,")));
    assert_eq!(
        decode_unified_id(&encoded),
        is_base64_encoded_unified_id(&encoded, "litellm_proxy:")
    );
}

#[test]
fn encoded_text_is_returned_whole() {
    assert_eq!(
        is_base64_encoded_unified_id(&URL_SAFE.encode(FILE_ID), "litellm_proxy:"),
        Some(FILE_ID.to_owned())
    );
    assert_eq!(
        decode_unified_id(&URL_SAFE_NO_PAD.encode(FILE_ID)),
        Some(FILE_ID.to_owned())
    );
    assert_eq!(
        decode_unified_id("bGl0ZWxsbV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ==A"),
        Some("litellm_proxy:file;unified_id,u\0".to_owned())
    );
}

#[rstest]
#[case::raw("file-1")]
#[case::invalid_utf8("/w==")]
#[case::one_more_than_a_multiple_of_four("A")]
#[case::empty("")]
#[case::only_junk("   ")]
#[case::non_ascii("\u{e9}")]
#[case::bang_after_unpadded("bGl0ZWxsbV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ!")]
#[case::space_after_unpadded("bGl0ZWxsbV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ ")]
#[case::pad_then_data("bGl0ZWxsbV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ=A")]
#[case::interior_pad_unpadded("bGl0=ZWxsbV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ")]
#[case::interior_pads_unpadded("bGl0==ZWxsbV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ")]
#[case::leading_pad("=bGl0ZWxsbV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ")]
#[case::wrong_prefix("b3RoZXI6ZmlsZTt1bmlmaWVkX2lkLHU=")]
fn undecodable_ids_are_absent(#[case] id: &str) {
    assert_eq!(is_base64_encoded_unified_id(id, "litellm_proxy:"), None);
    assert_eq!(decode_unified_id(id), None);
}

#[test]
fn prefix_argument_changes_detection_only() {
    let semicolon = URL_SAFE.encode("litellm_proxy;file");
    assert_eq!(
        is_base64_encoded_unified_id(&semicolon, "litellm_proxy:"),
        None
    );
    assert_eq!(
        is_base64_encoded_unified_id(&semicolon, "litellm_proxy;"),
        Some("litellm_proxy;file".to_owned())
    );
    assert_eq!(decode_unified_id(&semicolon), None);
}

#[rstest]
#[case::raw(VECTOR_STORE_ID.to_owned())]
#[case::padded(URL_SAFE.encode(VECTOR_STORE_ID))]
#[case::unpadded(URL_SAFE_NO_PAD.encode(VECTOR_STORE_ID))]
fn unified_field_projection(#[case] id: String) {
    let parts = UnifiedIdParts {
        resource_type: Some("vector_store".to_owned()),
        unified_uuid: Some("u".to_owned()),
        target_model_names: strings(&["a", "b", "", "a"]),
        provider_resource_id: Some("vs_1".to_owned()),
        model_id: Some("m".to_owned()),
    };
    assert_eq!(parse_unified_id(&id), Some(parts));
    assert_eq!(resource_type(&id), Some("vector_store".to_owned()));
    assert_eq!(unified_uuid(&id), Some("u".to_owned()));
    assert_eq!(target_model_names(&id), strings(&["a", "b", "", "a"]));
    assert_eq!(provider_resource_id(&id), Some("vs_1".to_owned()));
    assert_eq!(model_id(&id), Some("m".to_owned()));
}

#[test]
fn nonstring_inputs() {
    // the typed helper answers a null or integer value with exactly these empty results
    assert_eq!(is_base64_encoded_unified_id("", "litellm_proxy:"), None);
    assert_eq!(decode_unified_id(""), None);
    assert!(target_model_names("").is_empty());
    assert_eq!(resource_type(""), None);
    assert_eq!(unified_uuid(""), None);
    assert_eq!(model_id(""), None);
    assert_eq!(provider_resource_id(""), None);
    assert_eq!(parse_unified_id(""), None);
}

#[test]
fn bare_prefix_and_raw_text_parse() {
    let empty = UnifiedIdParts {
        resource_type: None,
        unified_uuid: None,
        target_model_names: Vec::new(),
        provider_resource_id: None,
        model_id: None,
    };
    assert_eq!(parse_unified_id("litellm_proxy:"), Some(empty));
    assert_eq!(parse_unified_id("file-1"), None);
    assert_eq!(parse_unified_id(&URL_SAFE.encode("file-1")), None);
    assert_eq!(parse_unified_id("xx litellm_proxy:file;unified_id,u"), None);
}

#[test]
fn captured_fields_are_stripped_like_python() {
    let id = "litellm_proxy: \u{1f}vector_store\u{3000};unified_id,\t u\n;target_model_names,\u{a0}a , \u{1c}b;resource_id, vs_1 ;model_id, m\u{2003}";
    let parts = parse_unified_id(id);
    assert_eq!(
        parts,
        Some(UnifiedIdParts {
            resource_type: Some("vector_store".to_owned()),
            unified_uuid: Some("u".to_owned()),
            target_model_names: strings(&["a", "b"]),
            provider_resource_id: Some("vs_1".to_owned()),
            model_id: Some("m".to_owned()),
        })
    );
    assert_eq!(unified_uuid("unified_id,  ;x"), Some(String::new()));
    assert_eq!(
        target_model_names("target_model_names, \u{200b}a\u{200b}"),
        strings(&["\u{200b}a\u{200b}"])
    );
}

#[rstest]
#[case::resource_first("resource_id,r;vector_store_id,v;file_id,f", "r")]
#[case::resource_last("file_id,f;vector_store_id,v;resource_id,r", "r")]
#[case::vector_store("vector_store_id,v;file_id,f", "v")]
#[case::file("file_id,f", "f")]
#[case::first_repeat("resource_id,one;resource_id,two", "one")]
#[case::empty_first_repeat("resource_id,;resource_id,two", "two")]
#[case::empty_resource_falls_back("resource_id,;file_id,f", "f")]
fn field_precedence(#[case] id: &str, #[case] expected: &str) {
    assert_eq!(provider_resource_id(id), Some(expected.to_owned()));
}

#[rstest]
#[case::file_encoding("llm_output_file_model_id,deploy", None)]
#[case::boundary(";model_id,m", Some("m"))]
#[case::start("model_id,m", Some("m"))]
#[case::after_file_field("llm_output_file_id,x;model_id,m", Some("m"))]
#[case::empty_then_boundary("model_id,;model_id,m", Some("m"))]
#[case::empty("litellm_proxy:file;model_id,;x", None)]
#[case::first_repeat("model_id,one;model_id,two", Some("one"))]
#[case::file_encoding_first("llm_output_file_model_id,deploy;model_id,m", Some("m"))]
#[case::space_before_boundary("a ;model_id,m", Some("m"))]
#[case::no_comma("model_id", None)]
fn model_id_anchoring(#[case] id: &str, #[case] expected: Option<&str>) {
    assert_eq!(model_id(id), expected.map(str::to_owned));
}

#[test]
fn unanchored_fields_come_from_arbitrary_raw_text() {
    assert_eq!(
        target_model_names("xtarget_model_names,a,b;c"),
        strings(&["a", "b"])
    );
    assert_eq!(unified_uuid("xunified_id,z"), Some("z".to_owned()));
    assert_eq!(
        resource_type("x litellm_proxy:file"),
        Some("file".to_owned())
    );
    assert_eq!(
        resource_type("litellm_proxy:;litellm_proxy:later"),
        Some("later".to_owned())
    );
    assert_eq!(parse_unified_id("x litellm_proxy:file"), None);
    assert!(target_model_names("target_model_names,;x").is_empty());
    assert!(target_model_names("nothing").is_empty());
}

#[test]
fn the_production_boundary_reports_both_reads() {
    let limits = ApiLimits::default();
    let encoded = URL_SAFE.encode(VECTOR_STORE_ID);
    let ids = decode_managed_resource_ids(encoded.as_bytes(), limits).unwrap();
    assert_eq!(ids.decoded.as_deref(), Some(VECTOR_STORE_ID));
    assert_eq!(ids.parts, parse_unified_id(VECTOR_STORE_ID));
    let raw = decode_managed_resource_ids(FILE_ID.as_bytes(), limits).unwrap();
    assert_eq!(raw.decoded, None);
    assert!(raw.parts.is_some());
    let other = decode_managed_resource_ids(b"file-1", limits).unwrap();
    assert_eq!(
        other,
        ManagedResourceIds {
            decoded: None,
            parts: None
        }
    );
}

#[test]
fn the_production_boundary_enforces_limits_and_utf8() {
    let limits = ApiLimits {
        max_json_bytes: 8,
        max_event_bytes: usize::MAX,
        max_items: usize::MAX,
    };
    assert_eq!(
        decode_managed_resource_ids(b"12345678", limits),
        Ok(ManagedResourceIds {
            decoded: None,
            parts: None
        })
    );
    assert_eq!(
        decode_managed_resource_ids(b"123456789", limits),
        Err(ManagedResourceIdsError::Limit(
            ApiLimitError::JsonTooLarge {
                observed: 9,
                limit: 8
            }
        ))
    );
    let error = decode_managed_resource_ids(&[0xff], ApiLimits::default()).unwrap_err();
    assert!(matches!(error, ManagedResourceIdsError::NotUtf8 { .. }));
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/managed_resource_ids");
    let replayed = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            drop(decode_managed_resource_ids(&bytes, ApiLimits::default()));
        })
        .count();
    assert!(replayed >= 10);
}
