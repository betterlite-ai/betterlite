#![cfg(test)]

use std::fs;

use rstest::rstest;

use super::CodecDecoding;
use super::CodecError;
use super::ResponseIdCodecKind;
use super::container::{build_container_id, decode_container_id};
use super::decode_response_id_codecs;
use super::encrypted::{
    EncryptedItemId, build_encrypted_item_id, decode_encrypted_item_id, unwrap_encrypted_content,
    wrap_encrypted_content,
};
use super::lenient::{Base64Error, TextDecodeError, decode_text};
use super::response::{
    build_response_id, decode_previous_response_id, decode_response_id,
    get_model_id_from_response_id, is_encoded_response_id,
};
use crate::api::api_boundaries::ApiLimits;

const PROVIDER_ENVELOPE: &str = "bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOm9wZW5haTttb2RlbF9pZDpkZXBfYTtyZXNwb25zZV9pZDpyZXNwX25hdGl2ZQ==";
const NULL_ENVELOPE: &str =
    "bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOk5vbmU7bW9kZWxfaWQ6Tm9uZTtyZXNwb25zZV9pZDpyZXNwX251bGw=";
const EMPTY_ENVELOPE: &str =
    "bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOjttb2RlbF9pZDo7cmVzcG9uc2VfaWQ6cmVzcF9lbXB0eQ==";
const SEMICOLON_ENVELOPE: &str =
    "bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOm9wZW5haTttb2RlbF9pZDpkZXBfYTtyZXNwb25zZV9pZDphO2I=";
const THREE_PART_ENVELOPE: &str = "YTtiO2M=";
const EMBEDDED_LABELS_ENVELOPE: &str =
    "eGxpdGVsbG06Y3VzdG9tX2xsbV9wcm92aWRlcjpwO2Ftb2RlbF9pZDptO2JyZXNwb25zZV9pZDpyZXNwb25zZV9pZDpy";

const ITEM_ENVELOPE: &str = "bGl0ZWxsbTptb2RlbF9pZDpkZXBfYTtpdGVtX2lkOnJzX2E7Yg==";
const ITEM_ENVELOPE_UNPADDED: &str = "bGl0ZWxsbTptb2RlbF9pZDpkZXBfYTtpdGVtX2lkOnJzX2E7Yg";
const ITEM_PLAIN_ENVELOPE: &str = "YTti";
const ITEM_EMBEDDED_ENVELOPE: &str = "bGl0ZWxsbTptb2RlbF9pZDpkZXBfYTtpdGVtX2lkOnJzX3hpdGVtX2lkOnk=";
const ITEM_EMBEDDED_MODEL_ENVELOPE: &str = "eGxpdGVsbG06bW9kZWxfaWQ6YTti";
const ITEM_EMPTY_ENVELOPE: &str = "YTs=";

const CONTENT_METADATA: &str = "bW9kZWxfaWQ6ZGVwX2E=";
const CONTENT_METADATA_UNPADDED: &str = "bW9kZWxfaWQ6ZGVwX2E";
const CONTENT_DOUBLE_METADATA: &str = "bW9kZWxfaWQ6YW1vZGVsX2lkOmI=";
const CONTENT_REPEATED_PREFIX: &str = "litellm_enc:litellm_enc:bW9kZWxfaWQ6YQ==;c";

const CONTAINER_NULL_ENVELOPE: &str =
    "bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOjttb2RlbF9pZDo7Y29udGFpbmVyX2lkOmM7YQ==";
const CONTAINER_NORMAL_ENVELOPE: &str =
    "bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOm9wZW5haTttb2RlbF9pZDpkZXBfYTtjb250YWluZXJfaWQ6YzI=";
const CONTAINER_SEMICOLON_ENVELOPE: &str =
    "bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOnA7bW9kZWxfaWQ6bTtjb250YWluZXJfaWQ6Yztk";
const CONTAINER_TRAILING_NEWLINE_ENVELOPE: &str =
    "bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOnA7bW9kZWxfaWQ6bTtjb250YWluZXJfaWQ6Ywo=";

fn response_bytes(encoded: &str) -> Vec<u8> {
    format!("resp_{encoded}").into_bytes()
}

fn container_bytes(encoded: &str) -> Vec<u8> {
    format!("cntr_{encoded}").into_bytes()
}

fn item_bytes(encoded: &str) -> Vec<u8> {
    format!("encitem_{encoded}").into_bytes()
}

fn content_bytes(metadata: &str, content: &str) -> Vec<u8> {
    format!("litellm_enc:{metadata};{content}").into_bytes()
}

fn entry(kind: ResponseIdCodecKind, bytes: &[u8]) -> CodecDecoding {
    decode_response_id_codecs(
        kind,
        bytes,
        ApiLimits {
            max_json_bytes: 1_000_000,
            max_event_bytes: 1_000_000,
            max_items: 1,
        },
    )
    .unwrap()
}

#[test]
fn permissive_base64_decodes_quads_and_the_full_alphabet() {
    assert_eq!(decode_text("aGVsbG8=").unwrap(), "hello");
    assert_eq!(decode_text("77+/").unwrap(), "\u{ffff}");
    assert_eq!(decode_text("YWJjZGVm").unwrap(), "abcdef");
}

#[test]
fn permissive_base64_skips_bytes_off_the_alphabet() {
    assert_eq!(decode_text("a!G@V#s$b%G&8=").unwrap(), "hello");
}

#[test]
fn permissive_base64_accepts_extra_trailing_padding() {
    assert_eq!(decode_text("aGVsbG8====").unwrap(), "hello");
    assert_eq!(decode_text("====").unwrap(), "");
}

#[test]
fn permissive_base64_rejects_a_lone_final_character() {
    assert!(matches!(
        decode_text("a"),
        Err(TextDecodeError::Base64(Base64Error::SingleCharacter))
    ));
}

#[test]
fn permissive_base64_requires_the_final_group_padding() {
    assert!(matches!(
        decode_text("aGVsbG8"),
        Err(TextDecodeError::Base64(Base64Error::MissingPadding))
    ));
    assert!(matches!(
        decode_text("AA="),
        Err(TextDecodeError::Base64(Base64Error::MissingPadding))
    ));
    assert_eq!(decode_text("AA==").unwrap(), "\0");
    assert_eq!(decode_text("AAA=").unwrap(), "\0\0");
}

#[test]
fn permissive_base64_rejects_a_payload_that_is_not_utf8() {
    assert!(matches!(decode_text("/w=="), Err(TextDecodeError::Utf8(_))));
}

#[test]
fn response_builds_the_litellm_envelope() {
    assert_eq!(
        build_response_id(Some("openai"), Some("dep_a"), "resp_native"),
        format!("resp_{PROVIDER_ENVELOPE}")
    );
    assert_eq!(
        build_response_id(None, None, "resp_null"),
        format!("resp_{NULL_ENVELOPE}")
    );
    assert_eq!(
        build_response_id(Some(""), Some(""), "resp_empty"),
        format!("resp_{EMPTY_ENVELOPE}")
    );
    assert_eq!(
        build_response_id(Some("openai"), Some("dep_a"), "a;b"),
        format!("resp_{SEMICOLON_ENVELOPE}")
    );
}

#[test]
fn response_decodes_the_litellm_envelope() {
    let decoded = decode_response_id(&format!("resp_{PROVIDER_ENVELOPE}"));
    assert_eq!(decoded.custom_llm_provider, Some(Some("openai".to_owned())));
    assert_eq!(decoded.model_id, Some(Some("dep_a".to_owned())));
    assert_eq!(decoded.response_id, Some("resp_native".to_owned()));
    assert!(is_encoded_response_id(&format!("resp_{PROVIDER_ENVELOPE}")));
    assert_eq!(
        decode_previous_response_id(&format!("resp_{PROVIDER_ENVELOPE}")),
        "resp_native"
    );
}

#[test]
fn response_serializes_null_fields_as_literal_none() {
    let decoded = decode_response_id(&format!("resp_{NULL_ENVELOPE}"));
    assert_eq!(decoded.custom_llm_provider, Some(Some("None".to_owned())));
    assert_eq!(decoded.model_id, Some(Some("None".to_owned())));
    assert!(is_encoded_response_id(&format!("resp_{NULL_ENVELOPE}")));
    assert_eq!(
        get_model_id_from_response_id(Some(&format!("resp_{NULL_ENVELOPE}"))),
        Some("None".to_owned())
    );
}

#[test]
fn response_decodes_empty_fields_as_empty_strings() {
    let decoded = decode_response_id(&format!("resp_{EMPTY_ENVELOPE}"));
    assert_eq!(decoded.custom_llm_provider, Some(Some(String::new())));
    assert_eq!(decoded.model_id, Some(Some(String::new())));
    assert!(is_encoded_response_id(&format!("resp_{EMPTY_ENVELOPE}")));
    assert_eq!(
        get_model_id_from_response_id(Some(&format!("resp_{EMPTY_ENVELOPE}"))),
        None
    );
}

#[test]
fn response_truncates_the_original_id_at_the_third_part() {
    let decoded = decode_response_id(&format!("resp_{SEMICOLON_ENVELOPE}"));
    assert_eq!(decoded.response_id, Some("a".to_owned()));
    assert_eq!(
        decode_previous_response_id(&format!("resp_{SEMICOLON_ENVELOPE}")),
        "a"
    );
}

#[rstest]
#[case::junk_only("resp_!")]
#[case::two_parts("resp_b25lO3R3bw==")]
#[case::no_semicolon("resp_YWJjZGVm")]
#[case::missing_padding("resp_aGVsbG8")]
#[case::invalid_utf8("resp_/w==")]
#[case::unknown_alphabet("resp_\u{e9}")]
fn response_falls_back_to_the_original_id(#[case] response_id: &str) {
    let decoded = decode_response_id(response_id);
    assert_eq!(decoded.custom_llm_provider, Some(None));
    assert_eq!(decoded.model_id, Some(None));
    assert_eq!(decoded.response_id, Some(response_id.to_owned()));
    assert!(!is_encoded_response_id(response_id));
    assert_eq!(decode_previous_response_id(response_id), response_id);
    assert_eq!(get_model_id_from_response_id(Some(response_id)), None);
}

#[test]
fn response_accepts_a_decoded_id_without_field_labels() {
    let decoded = decode_response_id(&format!("resp_{THREE_PART_ENVELOPE}"));
    assert_eq!(decoded.custom_llm_provider, Some(Some("a".to_owned())));
    assert_eq!(decoded.model_id, Some(Some("b".to_owned())));
    assert_eq!(decoded.response_id, Some("c".to_owned()));
}

#[test]
fn response_strips_each_label_throughout_its_segment() {
    let decoded = decode_response_id(&format!("resp_{EMBEDDED_LABELS_ENVELOPE}"));
    assert_eq!(decoded.custom_llm_provider, Some(Some("xp".to_owned())));
    assert_eq!(decoded.model_id, Some(Some("am".to_owned())));
    assert_eq!(decoded.response_id, Some("br".to_owned()));
}

#[test]
fn response_removes_every_prefix_occurrence() {
    let decoded = decode_response_id(&format!("resp_resp_{THREE_PART_ENVELOPE}"));
    assert_eq!(decoded.custom_llm_provider, Some(Some("a".to_owned())));
    assert_eq!(decoded.model_id, Some(Some("b".to_owned())));
    assert_eq!(decoded.response_id, Some("c".to_owned()));
}

#[test]
fn model_lookup_answers_only_for_a_non_empty_model() {
    assert_eq!(get_model_id_from_response_id(None), None);
    assert_eq!(
        get_model_id_from_response_id(Some(&format!("resp_{PROVIDER_ENVELOPE}"))),
        Some("dep_a".to_owned())
    );
}

#[test]
fn item_builds_the_litellm_envelope() {
    assert_eq!(
        build_encrypted_item_id("dep_a", "rs_a;b"),
        format!("encitem_{ITEM_ENVELOPE}")
    );
}

#[test]
fn item_roundtrips_and_restores_missing_padding() {
    let built = build_encrypted_item_id("dep_a", "rs_a;b");
    assert_eq!(
        decode_encrypted_item_id(&built),
        Some(EncryptedItemId {
            model_id: "dep_a".to_owned(),
            item_id: "rs_a;b".to_owned(),
        })
    );
    assert_eq!(
        decode_encrypted_item_id(&format!("encitem_{ITEM_ENVELOPE_UNPADDED}")),
        Some(EncryptedItemId {
            model_id: "dep_a".to_owned(),
            item_id: "rs_a;b".to_owned(),
        })
    );
}

#[test]
fn item_decodes_plain_and_embedded_parts() {
    assert_eq!(
        decode_encrypted_item_id(&format!("encitem_{ITEM_PLAIN_ENVELOPE}")),
        Some(EncryptedItemId {
            model_id: "a".to_owned(),
            item_id: "b".to_owned(),
        })
    );
    assert_eq!(
        decode_encrypted_item_id(&format!("encitem_{ITEM_EMBEDDED_ENVELOPE}")),
        Some(EncryptedItemId {
            model_id: "dep_a".to_owned(),
            item_id: "rs_xy".to_owned(),
        })
    );
    assert_eq!(
        decode_encrypted_item_id(&format!("encitem_{ITEM_EMBEDDED_MODEL_ENVELOPE}")),
        Some(EncryptedItemId {
            model_id: "xa".to_owned(),
            item_id: "b".to_owned(),
        })
    );
    assert_eq!(
        decode_encrypted_item_id(&format!("encitem_{ITEM_EMPTY_ENVELOPE}")),
        Some(EncryptedItemId {
            model_id: "a".to_owned(),
            item_id: String::new(),
        })
    );
}

#[rstest]
#[case::wrong_prefix("resp_abcd")]
#[case::lone_character("encitem_a")]
#[case::invalid_utf8("encitem_/w==")]
#[case::no_semicolon("encitem_bm9zZW1pY29sb24=")]
#[case::junk_metadata("encitem_!!")]
fn item_returns_none_without_an_error(#[case] encoded_id: &str) {
    assert_eq!(decode_encrypted_item_id(encoded_id), None);
}

#[test]
fn content_wraps_the_metadata_before_the_payload() {
    assert_eq!(
        wrap_encrypted_content("cipher;a", "dep_a"),
        format!("litellm_enc:{CONTENT_METADATA};cipher;a")
    );
}

#[test]
fn content_roundtrips_and_restores_missing_padding() {
    let wrapped = wrap_encrypted_content("cipher;a", "dep_a");
    assert_eq!(
        unwrap_encrypted_content(&wrapped),
        (Some("dep_a".to_owned()), "cipher;a".to_owned())
    );
    assert_eq!(
        unwrap_encrypted_content(&format!("litellm_enc:{CONTENT_METADATA_UNPADDED};cipher;a")),
        (Some("dep_a".to_owned()), "cipher;a".to_owned())
    );
}

#[test]
fn content_unwraps_an_empty_or_repeated_metadata_segment() {
    assert_eq!(
        unwrap_encrypted_content("litellm_enc:;cipher"),
        (Some(String::new()), "cipher".to_owned())
    );
    assert_eq!(
        unwrap_encrypted_content("litellm_enc:;"),
        (Some(String::new()), String::new())
    );
    assert_eq!(
        unwrap_encrypted_content(&format!("litellm_enc:{CONTENT_DOUBLE_METADATA};c")),
        (Some("ab".to_owned()), "c".to_owned())
    );
    assert_eq!(
        unwrap_encrypted_content(CONTENT_REPEATED_PREFIX),
        (Some("a".to_owned()), "c".to_owned())
    );
}

#[rstest]
#[case::unwrapped("plain-cipher")]
#[case::absent_delimiter("litellm_enc:bW9kZWxfaWQ6ZGVwX2E=")]
#[case::invalid_utf8("litellm_enc:/w==;cipher")]
#[case::invalid_padding("litellm_enc:a;cipher")]
fn content_returns_the_input_when_it_cannot_unwrap(#[case] wrapped_content: &str) {
    assert_eq!(
        unwrap_encrypted_content(wrapped_content),
        (None, wrapped_content.to_owned())
    );
}

#[test]
fn container_builds_empty_fields_for_missing_metadata() {
    assert_eq!(
        build_container_id(None, None, "c;a"),
        format!("cntr_{CONTAINER_NULL_ENVELOPE}")
    );
    assert_eq!(
        build_container_id(Some("openai"), Some("dep_a"), "c2"),
        format!("cntr_{CONTAINER_NORMAL_ENVELOPE}")
    );
}

#[test]
fn container_decodes_empty_and_legacy_none_fields_as_null() {
    let null = decode_container_id(&format!("cntr_{CONTAINER_NULL_ENVELOPE}"));
    assert_eq!(null.custom_llm_provider, Some(None));
    assert_eq!(null.model_id, Some(None));
    assert_eq!(null.response_id, Some("c;a".to_owned()));

    let legacy = decode_container_id(&build_container_id(Some("None"), Some("None"), "c1"));
    assert_eq!(legacy.custom_llm_provider, Some(None));
    assert_eq!(legacy.model_id, Some(None));
    assert_eq!(legacy.response_id, Some("c1".to_owned()));

    let named = decode_container_id(&format!("cntr_{CONTAINER_NORMAL_ENVELOPE}"));
    assert_eq!(named.custom_llm_provider, Some(Some("openai".to_owned())));
    assert_eq!(named.model_id, Some(Some("dep_a".to_owned())));
    assert_eq!(named.response_id, Some("c2".to_owned()));
}

#[test]
fn container_keeps_semicolons_in_the_container_id() {
    let decoded = decode_container_id(&format!("cntr_{CONTAINER_SEMICOLON_ENVELOPE}"));
    assert_eq!(decoded.response_id, Some("c;d".to_owned()));
}

#[test]
fn container_accepts_one_trailing_newline() {
    let decoded = decode_container_id(&format!("cntr_{CONTAINER_TRAILING_NEWLINE_ENVELOPE}"));
    assert_eq!(decoded.custom_llm_provider, Some(Some("p".to_owned())));
    assert_eq!(decoded.model_id, Some(Some("m".to_owned())));
    assert_eq!(decoded.response_id, Some("c".to_owned()));
}

#[rstest]
#[case::wrong_prefix("resp_abcd")]
#[case::missing_litellm_prefix(
    "cntr_bm9wZTpjdXN0b21fbGxtX3Byb3ZpZGVyOnA7bW9kZWxfaWQ6bTtjb250YWluZXJfaWQ6Yw=="
)]
#[case::malformed_labels(
    "cntr_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOnA7bW9kZWxfaWQ6bTt3cm9uZ19pZDpj"
)]
#[case::empty_container(
    "cntr_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOnA7bW9kZWxfaWQ6bTtjb250YWluZXJfaWQ6"
)]
#[case::provider_with_semicolon(
    "cntr_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOnA7cTttb2RlbF9pZDptO2NvbnRhaW5lcl9pZDpj"
)]
#[case::model_with_semicolon(
    "cntr_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOnA7bW9kZWxfaWQ6bTtuO2NvbnRhaW5lcl9pZDpj"
)]
#[case::inner_newline(
    "cntr_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOnA7bW9kZWxfaWQ6bTtjb250YWluZXJfaWQ6Ywpk"
)]
#[case::missing_padding(
    "cntr_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOm9wZW5haTttb2RlbF9pZDpkZXBfYTtjb250YWluZXJfaWQ6YzI"
)]
fn container_returns_the_original_for_malformed_ids(#[case] container_id: &str) {
    let decoded = decode_container_id(container_id);
    assert_eq!(decoded.custom_llm_provider, Some(None));
    assert_eq!(decoded.model_id, Some(None));
    assert_eq!(decoded.response_id, Some(container_id.to_owned()));
}

#[test]
fn selector_covers_every_byte() {
    assert_eq!(
        ResponseIdCodecKind::from_selector(0),
        ResponseIdCodecKind::Response
    );
    assert_eq!(
        ResponseIdCodecKind::from_selector(1),
        ResponseIdCodecKind::Container
    );
    assert_eq!(
        ResponseIdCodecKind::from_selector(2),
        ResponseIdCodecKind::EncryptedItem
    );
    assert_eq!(
        ResponseIdCodecKind::from_selector(3),
        ResponseIdCodecKind::EncryptedContent
    );
    assert_eq!(
        ResponseIdCodecKind::from_selector(4),
        ResponseIdCodecKind::Response
    );
    assert_eq!(
        ResponseIdCodecKind::from_selector(255),
        ResponseIdCodecKind::EncryptedContent
    );
}

#[test]
fn entry_decodes_the_response_selector() {
    assert_eq!(
        entry(
            ResponseIdCodecKind::Response,
            &response_bytes(PROVIDER_ENVELOPE)
        ),
        CodecDecoding {
            custom_llm_provider: Some("openai".to_owned()),
            model_id: Some("dep_a".to_owned()),
            restored_id: Some("resp_native".to_owned()),
        }
    );
}

#[test]
fn entry_decodes_the_container_selector() {
    assert_eq!(
        entry(
            ResponseIdCodecKind::Container,
            &container_bytes(CONTAINER_NORMAL_ENVELOPE)
        ),
        CodecDecoding {
            custom_llm_provider: Some("openai".to_owned()),
            model_id: Some("dep_a".to_owned()),
            restored_id: Some("c2".to_owned()),
        }
    );
}

#[test]
fn entry_decodes_the_item_selector() {
    assert_eq!(
        entry(
            ResponseIdCodecKind::EncryptedItem,
            &item_bytes(ITEM_ENVELOPE)
        ),
        CodecDecoding {
            custom_llm_provider: None,
            model_id: Some("dep_a".to_owned()),
            restored_id: Some("rs_a;b".to_owned()),
        }
    );
}

#[test]
fn entry_decodes_the_content_selector() {
    assert_eq!(
        entry(
            ResponseIdCodecKind::EncryptedContent,
            &content_bytes(CONTENT_METADATA, "cipher;a")
        ),
        CodecDecoding {
            custom_llm_provider: None,
            model_id: Some("dep_a".to_owned()),
            restored_id: Some("cipher;a".to_owned()),
        }
    );
}

#[test]
fn entry_falls_back_with_each_selector() {
    assert_eq!(
        entry(ResponseIdCodecKind::Response, b"resp_!"),
        CodecDecoding {
            custom_llm_provider: None,
            model_id: None,
            restored_id: Some("resp_!".to_owned()),
        }
    );
    assert_eq!(
        entry(ResponseIdCodecKind::Container, b"resp_abcd"),
        CodecDecoding {
            custom_llm_provider: None,
            model_id: None,
            restored_id: Some("resp_abcd".to_owned()),
        }
    );
    assert_eq!(
        entry(ResponseIdCodecKind::EncryptedItem, b"encitem_a"),
        CodecDecoding {
            custom_llm_provider: None,
            model_id: None,
            restored_id: None,
        }
    );
    assert_eq!(
        entry(
            ResponseIdCodecKind::EncryptedContent,
            b"litellm_enc:a;cipher"
        ),
        CodecDecoding {
            custom_llm_provider: None,
            model_id: None,
            restored_id: Some("litellm_enc:a;cipher".to_owned()),
        }
    );
}

#[test]
fn entry_handles_bytes_that_are_not_utf8() {
    assert_eq!(
        entry(ResponseIdCodecKind::Response, b"resp_\xff"),
        CodecDecoding {
            custom_llm_provider: None,
            model_id: None,
            restored_id: None,
        }
    );
    assert_eq!(
        entry(ResponseIdCodecKind::Container, b"cntr_\xff"),
        CodecDecoding {
            custom_llm_provider: None,
            model_id: None,
            restored_id: None,
        }
    );
    assert_eq!(
        entry(ResponseIdCodecKind::EncryptedItem, b"encitem_\xff"),
        CodecDecoding {
            custom_llm_provider: None,
            model_id: None,
            restored_id: None,
        }
    );
    assert_eq!(
        entry(ResponseIdCodecKind::EncryptedContent, b"litellm_enc:\xff;c"),
        CodecDecoding {
            custom_llm_provider: None,
            model_id: None,
            restored_id: None,
        }
    );
}

#[rstest]
#[case::response(ResponseIdCodecKind::Response)]
#[case::container(ResponseIdCodecKind::Container)]
#[case::encrypted_item(ResponseIdCodecKind::EncryptedItem)]
#[case::encrypted_content(ResponseIdCodecKind::EncryptedContent)]
fn entry_enforces_the_byte_limit(#[case] kind: ResponseIdCodecKind) {
    let bytes = b"resp_abcd";
    let exact = ApiLimits {
        max_json_bytes: bytes.len(),
        max_event_bytes: bytes.len(),
        max_items: 1,
    };
    decode_response_id_codecs(kind, bytes, exact).unwrap();

    let short_json = ApiLimits {
        max_json_bytes: bytes.len().saturating_sub(1),
        max_event_bytes: bytes.len(),
        max_items: 1,
    };
    assert_eq!(
        decode_response_id_codecs(kind, bytes, short_json).unwrap_err(),
        CodecError::TooLarge {
            observed: bytes.len(),
            limit: bytes.len().saturating_sub(1),
        }
    );

    let short_event = ApiLimits {
        max_json_bytes: bytes.len(),
        max_event_bytes: bytes.len().saturating_sub(1),
        max_items: 1,
    };
    assert_eq!(
        decode_response_id_codecs(kind, bytes, short_event).unwrap_err(),
        CodecError::TooLarge {
            observed: bytes.len(),
            limit: bytes.len().saturating_sub(1),
        }
    );
}

#[test]
fn replays_corpus() {
    let corpus = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fuzz/corpus/response_id_codecs"
    );
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .for_each(|path| {
            let bytes = fs::read(path).unwrap();
            let (selector, id) = bytes.split_first().unwrap();
            let kind = ResponseIdCodecKind::from_selector(*selector);
            assert!(
                decode_response_id_codecs(kind, id, ApiLimits::default()).is_ok(),
                "the seeded id decodes"
            );
        });
}
