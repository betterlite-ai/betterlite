use super::*;
use betterlite_error::error::class_message;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_wrapper::error::{class_name, class_status};

use crate::skill_calls::error::{NO_DATABASE, NativeErrorPayload, no_database_failure};

fn arguments(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

fn field(marker: &str, blob: u64) -> String {
    format!(r#"{{"$file": {{"name": "{marker}", "blob": {blob}}}}}"#)
}

#[test]
fn request_text_is_a_json_object() {
    let parsed = parse(r#"{"skill_id": "s1"}"#).unwrap();
    assert_eq!(parsed.get("skill_id"), Some(&Json::Text("s1".to_owned())));
    parse("[1, 2]").unwrap_err();
}

#[test]
fn request_skill_arguments_split_named_parameters() {
    let parsed = skill_arguments(arguments(
        r#"{"skill_id": "s1", "api_key": "k", "api_base": "https://x", "extra_headers": {"x-a": "b"}, "extra_query": {"q": 1}, "timeout": 3, "custom_llm_provider": "anthropic", "metadata": {"m": true}}"#,
    ))
    .unwrap();
    assert_eq!(parsed.skill_id, "s1");
    assert_eq!(parsed.params.provider(), "anthropic");
    assert!(parsed.params.api_key.is_some());
    assert_eq!(parsed.params.api_base.as_deref(), Some("https://x"));
    assert_eq!(parsed.params.extra_headers.get("x-a").unwrap(), "b");
    assert_eq!(parsed.params.timeout, Some(Duration::from_secs(3)));
    assert_eq!(parsed.params.extra_query, Some(arguments(r#"{"q": 1}"#)));
    assert_eq!(parsed.litellm_call_id, None);
    assert!(parsed.kwargs.contains_key("api_key"));
    assert!(parsed.kwargs.contains_key("metadata"));
    assert!(!parsed.kwargs.contains_key("skill_id"));
    assert!(!parsed.kwargs.contains_key("extra_headers"));
    assert!(!parsed.kwargs.contains_key("timeout"));
    assert!(!parsed.kwargs.contains_key("custom_llm_provider"));
}

#[test]
fn request_skill_arguments_default_provider_is_anthropic() {
    let parsed = skill_arguments(arguments(r#"{"skill_id": "s1"}"#)).unwrap();
    assert_eq!(parsed.params.provider(), "anthropic");
    assert_eq!(
        parsed.params.custom_llm_provider.as_deref(),
        Some("anthropic")
    );
}

#[test]
fn request_skill_arguments_require_a_skill_id() {
    let Err(RequestError::FieldType { provider, name }) = skill_arguments(arguments(r#"{}"#))
    else {
        panic!("a missing skill_id is a field error");
    };
    assert_eq!(provider, "anthropic");
    assert_eq!(name, "skill_id");
}

#[test]
fn request_skill_arguments_carry_the_named_provider() {
    let Err(RequestError::FieldType { provider, name }) = skill_arguments(arguments(
        r#"{"skill_id": 7, "custom_llm_provider": "openai"}"#,
    )) else {
        panic!("a skill_id that is not text is a field error");
    };
    assert_eq!(provider, "openai");
    assert_eq!(name, "skill_id");
}

#[test]
fn request_create_arguments_split_kwargs() {
    let parsed = create_arguments(arguments(
        r#"{"display_title": "T", "files": [{"$file": {"blob": 0}}], "extra_body": {"description": "D"}, "timeout": 9, "user_id": "u", "api_key": "k"}"#,
    ))
    .unwrap();
    assert_eq!(parsed.display_title.as_deref(), Some("T"));
    assert_eq!(parsed.files.as_ref().map(Vec::len), Some(1));
    assert_eq!(
        parsed
            .extra_body
            .as_ref()
            .and_then(|body| body.get("description")),
        Some(&Json::Text("D".to_owned()))
    );
    assert_eq!(parsed.params.timeout, Some(Duration::from_secs(9)));
    assert!(parsed.kwargs.contains_key("user_id"));
    assert!(parsed.kwargs.contains_key("api_key"));
    assert!(!parsed.kwargs.contains_key("display_title"));
    assert!(!parsed.kwargs.contains_key("files"));
    assert!(!parsed.kwargs.contains_key("extra_body"));
}

#[test]
fn request_create_arguments_without_files() {
    let parsed = create_arguments(arguments(r#"{"display_title": "T", "files": null}"#)).unwrap();
    assert!(parsed.files.is_none());
}

#[test]
fn request_create_arguments_reject_files_of_the_wrong_type() {
    let Err(RequestError::FieldType { name, .. }) =
        create_arguments(arguments(r#"{"files": "a.txt"}"#))
    else {
        panic!("files that are not a list are a field error");
    };
    assert_eq!(name, "files");
}

#[test]
fn request_list_arguments_fields() {
    let parsed = list_arguments(arguments(
        r#"{"limit": 5, "page": "p", "source": "s", "api_base": "https://x", "metadata": {}}"#,
    ))
    .unwrap();
    assert_eq!(parsed.limit, Some(5));
    assert_eq!(parsed.page.as_deref(), Some("p"));
    assert_eq!(parsed.source.as_deref(), Some("s"));
    assert!(parsed.kwargs.contains_key("api_base"));
    assert!(parsed.kwargs.contains_key("metadata"));
    assert!(!parsed.kwargs.contains_key("limit"));
}

#[test]
fn request_list_arguments_reject_a_limit_of_the_wrong_type() {
    let Err(RequestError::FieldType { provider, name }) =
        list_arguments(arguments(r#"{"limit": "5"}"#))
    else {
        panic!("a limit that is not a number is a field error");
    };
    assert_eq!(provider, "anthropic");
    assert_eq!(name, "limit");
}

#[test]
fn request_extra_headers_become_a_header_map() {
    let parsed = skill_arguments(arguments(
        r#"{"skill_id": "s1", "extra_headers": {"x-a": "b", "anthropic-beta": "c"}}"#,
    ))
    .unwrap();
    assert_eq!(parsed.params.extra_headers.get("x-a").unwrap(), "b");
    assert_eq!(
        parsed.params.extra_headers.get("anthropic-beta").unwrap(),
        "c"
    );
    let Err(RequestError::FieldType { name, .. }) = skill_arguments(arguments(
        r#"{"skill_id": "s1", "extra_headers": {"x-a": 3}}"#,
    )) else {
        panic!("a header that is not text is a field error");
    };
    assert_eq!(name, "extra_headers");
}

#[test]
fn request_extra_query_must_be_an_object() {
    let parsed = list_arguments(arguments(
        r#"{"extra_query": {"limit": 2, "unknown": "x"}}"#,
    ))
    .unwrap();
    assert_eq!(
        parsed.params.extra_query,
        Some(arguments(r#"{"limit": 2, "unknown": "x"}"#))
    );
    let Err(RequestError::FieldType { name, .. }) =
        list_arguments(arguments(r#"{"extra_query": "limit=2"}"#))
    else {
        panic!("an extra_query that is not an object is a field error");
    };
    assert_eq!(name, "extra_query");
}

#[test]
fn request_timeout_is_seconds() {
    let parsed = skill_arguments(arguments(r#"{"skill_id": "s1", "timeout": 0.5}"#)).unwrap();
    assert_eq!(parsed.params.timeout, Some(Duration::from_millis(500)));
}

#[test]
fn request_timeout_rejects_text() {
    let text = r#"{"skill_id": "s1", "timeout": "5"}"#;
    let Err(RequestError::FieldType { name, .. }) = skill_arguments(arguments(text)) else {
        panic!("a timeout that is text is a field error");
    };
    assert_eq!(name, "timeout");
}

#[test]
fn request_timeout_rejects_a_negative_number() {
    let text = r#"{"skill_id": "s1", "timeout": -1}"#;
    let Err(RequestError::FieldType { name, .. }) = skill_arguments(arguments(text)) else {
        panic!("a negative timeout is a field error");
    };
    assert_eq!(name, "timeout");
}

#[test]
fn request_timeout_rejects_a_flag() {
    let text = r#"{"skill_id": "s1", "timeout": true}"#;
    let Err(RequestError::FieldType { name, .. }) = skill_arguments(arguments(text)) else {
        panic!("a timeout that is a flag is a field error");
    };
    assert_eq!(name, "timeout");
}

#[test]
fn request_files_resolve_markers() {
    let marker = field("a.txt", 0);
    let named = r#"{"$file": {"name": "b.bin", "blob": 1, "content_type": "application/pdf"}}"#;
    let items: Vec<Json> = serde_json::from_str(&format!("[{marker}, {named}]")).unwrap();
    let mut blobs = vec![b"one".to_vec(), b"two".to_vec()];
    let resolved = files("anthropic", &items, &mut blobs).unwrap();
    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[0].name, "a.txt");
    assert_eq!(resolved[0].content_type, None);
    assert_eq!(resolved[0].bytes, b"one");
    assert_eq!(resolved[1].name, "b.bin");
    assert_eq!(resolved[1].content_type.as_deref(), Some("application/pdf"));
    assert_eq!(resolved[1].bytes, b"two");
    assert!(blobs[0].is_empty());
}

#[test]
fn request_files_reject_plain_values() {
    let items: Vec<Json> = serde_json::from_str(r#"[{"name": "a.txt"}, "b.txt"]"#).unwrap();
    let mut blobs = Vec::new();
    let Err(RequestError::FieldType { name, .. }) = files("anthropic", &items, &mut blobs) else {
        panic!("a file that is not a $file marker is a field error");
    };
    assert_eq!(name, "files");
}

#[test]
fn request_files_reject_objects_without_a_marker() {
    let items: Vec<Json> = serde_json::from_str(r#"[{"name": "a.txt"}]"#).unwrap();
    let mut blobs = Vec::new();
    let Err(RequestError::FieldType { name, .. }) = files("anthropic", &items, &mut blobs) else {
        panic!("an object without $file is a field error, not an ignored entry");
    };
    assert_eq!(name, "files");
}

#[test]
fn error_field_type_is_a_bad_request_naming_the_field() {
    let error = RequestError::field("anthropic", "limit");
    let class = error.class();
    assert_eq!(class_name(&class), "BadRequestError");
    assert_eq!(class_status(&class), Some(400));
    assert_eq!(
        class_message(&class),
        "the limit field does not have the declared type"
    );
    let payload = NativeErrorPayload::from_class(&class);
    assert_eq!(payload.kind, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
    assert_eq!(payload.llm_provider.as_deref(), Some("anthropic"));
    assert_eq!(payload.num_retries, None);
}

#[test]
fn error_no_database_is_a_connection_error() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let failure = no_database_failure(&redactor);
    assert_eq!(class_name(&failure.class), "APIConnectionError");
    let message = class_message(&failure.class);
    assert!(message.contains(NO_DATABASE));
    assert!(message.contains("Litellm_proxyException"));
}
