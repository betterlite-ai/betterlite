use std::collections::BTreeMap;

use crate::batch_embeddings::classify::BatchRecordKind;
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::CreatePurpose;
use betterlite_prompts::media_metadata::{FileBody, FileData};

use super::support::{
    ACCESS_KEY, FIXED_UUID, Families, GATEWAY_MODEL, SECRET_KEY, batch_file, booking_params,
    chat_body_of, error_of, json, params, record, run_upload, run_upload_missing_file, transform,
    transform_with, value,
};
use crate::batch_records::chat_body::text_completion_to_chat;
use crate::batch_records::error::BatchRecordsError;
use crate::batch_records::record::{LINE_BOUNDARIES, OpenAiBatchRecord};
use crate::request_policy::RequestValue;

pub(super) fn chat_batch_record() {
    let families = Families::default();
    let records = transform(
        &families,
        r#"{"custom_id":"a","url":"/v1/chat/completions","body":{"model":"openai.gpt-oss-20b-1:0","messages":[{"role":"developer","content":"be nice"},{"content":"hi","role":"user"}],"max_tokens":16}}"#,
        "",
    )
    .expect("the chat record transforms");
    assert_eq!(
        value(&records[0]),
        serde_json::json!({
            "recordId": "a",
            "modelInput": {
                "messages": [
                    {"role": "system", "content": "be nice"},
                    {"content": "hi", "role": "user"}
                ],
                "max_tokens": 16
            }
        })
    );
    // a passthrough provider names no family, so nothing was mapped for it
    assert!(families.nova_calls().is_empty());
    assert!(families.anthropic_params_calls().is_empty());
    assert!(families.responses_calls().is_empty());

    // the anthropic family maps through the invoke route model and the filtered body params
    let mut anthropic = Families::default();
    anthropic.anthropic_params = Some(params(r#"{"max_tokens":256,"json_mode":true}"#));
    anthropic.anthropic_body = Some(json(
        r#"{"anthropic_version":"bedrock-2023-05-31","messages":[]}"#,
    ));
    let anthropic_records = transform(
        &anthropic,
        r#"{"custom_id":"t","url":"/v1/chat/completions","body":{"model":"anthropic.claude-3-5-sonnet-20241022-v2:0","messages":[{"role":"developer","content":"be nice"},{"content":"hi","role":"user"}],"max_tokens":256,"top_k":5,"allowed_openai_params":["top_k"],"api_key":"sk-fixture"}}"#,
        "",
    )
    .expect("the anthropic record transforms");
    assert_eq!(
        value(&anthropic_records[0]),
        serde_json::json!({
            "recordId": "t",
            "modelInput": {"anthropic_version": "bedrock-2023-05-31", "messages": []}
        })
    );
    let params_calls = anthropic.anthropic_params_calls();
    assert_eq!(
        params_calls[0].0,
        "invoke/anthropic.claude-3-5-sonnet-20241022-v2:0"
    );
    // api_key is dropped, and allowed_openai_params is the one owned kwarg the mapper takes
    assert_eq!(
        params_calls[0].1,
        params(r#"{"max_tokens":256,"top_k":5,"allowed_openai_params":["top_k"]}"#)
    );
    let body_calls = anthropic.anthropic_body_calls();
    assert_eq!(
        body_calls[0].model,
        "anthropic.claude-3-5-sonnet-20241022-v2:0"
    );
    assert_eq!(
        value(&Json::List(body_calls[0].messages.clone())),
        serde_json::json!([
            {"role": "system", "content": "be nice"},
            {"content": "hi", "role": "user"}
        ])
    );
    // the mapper adds json_mode and the request handler strips it before the body transformer
    assert_eq!(
        body_calls[0].optional_params,
        params(r#"{"max_tokens":256}"#)
    );
}

pub(super) fn text_completion_to_chat_case() {
    let families = Families::default();
    let records = transform(
        &families,
        r#"{"custom_id":"c","url":"/v1/completions","body":{"model":"qwen.qwen3-32b-v1:0","prompt":"hello"}}"#,
        "",
    )
    .expect("the completion record transforms");
    assert_eq!(
        value(&records[0]),
        serde_json::json!({
            "recordId": "c",
            "modelInput": {"messages": [{"content": "hello", "role": "user"}]}
        })
    );
    let error = error_of(transform(
        &families,
        r#"{"url":"/v1/completions","body":{"model":"qwen.qwen3-32b-v1:0"}}"#,
        "",
    ));
    assert_eq!(
        error.to_string(),
        "Batch record for /v1/completions is missing required `prompt` field: model=qwen.qwen3-32b-v1:0"
    );
    let null_prompt = text_completion_to_chat(&json(r#"{"model":"m","prompt":null}"#))
        .expect_err("a null prompt is refused");
    assert!(
        null_prompt
            .to_string()
            .contains("missing required `prompt`")
    );
    // the shared prompt helper refuses an empty prompt, a token list and a mixed list
    let empty = text_completion_to_chat(&json(r#"{"model":"m","prompt":""}"#))
        .expect_err("an empty prompt is refused");
    assert_eq!(
        empty.to_string(),
        "`prompt` must be a non-empty string or a non-empty list of strings. Got: str."
    );
    let mixed = text_completion_to_chat(&json(r#"{"model":"m","prompt":["ok",""]}"#))
        .expect_err("a mixed list is refused");
    assert_eq!(
        mixed.to_string(),
        "`prompt` must be a non-empty string or a non-empty list of strings. Got: list."
    );
    let tokens = text_completion_to_chat(&json(r#"{"model":"m","prompt":[1,2]}"#))
        .expect_err("a token list is refused");
    assert_eq!(
        tokens.to_string(),
        "`prompt` must be a non-empty string or a non-empty list of strings. Got: list."
    );
    // a list of strings becomes one user message per element, in order
    let list = text_completion_to_chat(&json(r#"{"model":"m","prompt":["a","b"]}"#))
        .expect("a string list maps");
    assert_eq!(
        value(&list)["messages"],
        serde_json::json!([{"content": "a", "role": "user"}, {"content": "b", "role": "user"}])
    );
}

pub(super) fn responses_to_chat() {
    let mut families = Families::default();
    families.responses_body = Some(json(
        r#"{"messages":[{"role":"user","content":"hi"}],"max_output_tokens":5}"#,
    ));
    let source = json(
        r#"{"custom_id":"r","url":"/v1/responses","body":{"model":"qwen.qwen3-32b-v1:0","input":"hi","max_output_tokens":5}}"#,
    );
    let records = transform(
        &families,
        &serde_json::to_string(&value(&source)).expect("renders"),
        "",
    )
    .expect("the responses record transforms");
    assert_eq!(
        value(&records[0]),
        serde_json::json!({
            "recordId": "r",
            "modelInput": {
                "messages": [{"content": "hi", "role": "user"}],
                "max_output_tokens": 5
            }
        })
    );
    assert_eq!(families.responses_calls(), vec![source]);
}

pub(super) fn embedding_batch_record() {
    let families = Families::default();
    let records = transform(
        &families,
        r#"{"custom_id":"e","url":"/v1/embeddings","body":{"model":"amazon.titan-embed-text-v2:0","input":"hello"}}"#,
        "",
    )
    .expect("the embedding record transforms");
    assert_eq!(
        value(&records[0]),
        serde_json::json!({"recordId": "e", "modelInput": {"inputText": "hello"}})
    );
    // the chat families never see an embedding record
    assert!(families.nova_calls().is_empty());
    // an embedding body handed to the chat normalizer names the missing chat equivalent
    let refused = chat_body_of(&json(r#"{"model":"m"}"#), BatchRecordKind::Embedding)
        .expect_err("an embedding body has no chat counterpart");
    assert_eq!(
        refused.to_string(),
        "Embedding batch records do not have a chat-completion equivalent"
    );
}

pub(super) fn alias_and_target_model() {
    let aliased_families = nova_families();
    let aliases = BTreeMap::from([(
        String::from("my-alias"),
        String::from("amazon.nova-pro-v1:0"),
    )]);
    let aliased = record(
        r#"{"custom_id":"a","url":"/v1/chat/completions","body":{"model":"my-alias","messages":[{"content":"hi","role":"user"}]}}"#,
    );
    transform_with(&aliased_families, &[aliased], "", &aliases)
        .expect("the aliased record transforms");
    let calls = aliased_families.nova_calls();
    assert_eq!(calls[0].model, "amazon.nova-pro-v1:0");

    let target_families = nova_families();
    let unnamed = record(
        r#"{"custom_id":"b","url":"/v1/chat/completions","body":{"model":"my-custom","messages":[{"content":"hi","role":"user"}]}}"#,
    );
    transform_with(
        &target_families,
        &[unnamed],
        "amazon.nova-pro-v1:0",
        &BTreeMap::new(),
    )
    .expect("the target model resolves the record");
    assert_eq!(
        target_families.nova_calls()[0].model,
        "amazon.nova-pro-v1:0"
    );

    // a record that names its own provider keeps that model even when a target model exists
    let named_families = Families::default();
    let named = record(
        r#"{"custom_id":"c","url":"/v1/chat/completions","body":{"model":"openai.gpt-oss-20b-1:0","messages":[{"content":"hi","role":"user"}]}}"#,
    );
    transform_with(
        &named_families,
        &[named],
        "amazon.nova-pro-v1:0",
        &BTreeMap::new(),
    )
    .expect("the named record transforms");
    assert!(named_families.nova_calls().is_empty());

    // a record with no model names nothing, so only a target model can route it
    let model_less = record(
        r#"{"custom_id":"d","url":"/v1/chat/completions","body":{"messages":[{"content":"hi","role":"user"}]}}"#,
    );
    let model_less_families = nova_families();
    transform_with(
        &model_less_families,
        &[model_less],
        "amazon.nova-pro-v1:0",
        &BTreeMap::new(),
    )
    .expect("the model-less record transforms");
    assert_eq!(
        model_less_families.nova_calls()[0].model,
        "amazon.nova-pro-v1:0"
    );
}

pub(super) fn default_record_id() {
    let families = Families::default();
    let sources = [
        "/v1/chat/completions",
        "/v1/chat/completions",
        "/v1/chat/completions",
    ];
    let bodies = [
        r#"{"body":{"model":"openai.gpt-oss-20b-1:0","messages":[]}}"#,
        r#"{"custom_id":null,"body":{"model":"openai.gpt-oss-20b-1:0","messages":[]}}"#,
        r#"{"custom_id":"z","body":{"model":"openai.gpt-oss-20b-1:0","messages":[]}}"#,
    ];
    let records: Vec<OpenAiBatchRecord> = sources
        .iter()
        .zip(bodies)
        .map(|(url, body)| {
            let line = format!(r#"{{"url":"{url}",{body}"#);
            record(&line)
        })
        .collect();
    let transformed =
        transform_with(&families, &records, "", &BTreeMap::new()).expect("the records transform");
    assert_eq!(
        value(&transformed[0])["recordId"],
        value(&json(r#""CALL0000000""#))
    );
    assert_eq!(value(&transformed[1])["recordId"], serde_json::Value::Null);
    assert_eq!(value(&transformed[2])["recordId"], value(&json(r#""z""#)));
    // the width pads and never truncates
    assert_eq!(records[0].record_id(9), json(r#""CALL0000009""#));
    assert_eq!(records[0].record_id(10), json(r#""CALL0000010""#));
    assert_eq!(records[0].record_id(12_345_678), json(r#""CALL12345678""#));
}

pub(super) fn jsonl_blank_lines() {
    let families = Families::batch();
    let file = FileData::Tuple3 {
        filename: None,
        content: FileBody::Bytes(
            "\n  \u{1f}\n{\"custom_id\":\"a\",\"body\":{\"model\":\"openai.gpt-oss-20b-1:0\",\"messages\":[{\"content\":\"hi\",\"role\":\"user\"}]}}\r\n\u{2028}\n"
                .as_bytes()
                .to_vec(),
        ),
        content_type: Some(String::from("application/jsonl")),
    };
    let (booking, _) = run_upload(
        &families,
        &file,
        Some(CreatePurpose::Batch),
        "",
        &mut booking_params(),
    );
    let signed = booking.expect("the batch upload signs");
    assert_eq!(
        signed.data,
        "{\"recordId\": \"a\", \"modelInput\": {\"messages\": [{\"content\": \"hi\", \"role\": \"user\"}]}}"
    );
    let (empty, _) = run_upload(
        &Families::batch(),
        &FileData::Direct(FileBody::Bytes(b"\n\n".to_vec())),
        Some(CreatePurpose::Batch),
        "",
        &mut booking_params(),
    );
    assert_eq!(empty.expect("the empty batch upload signs").data, "");
    assert_eq!(LINE_BOUNDARIES.len(), 10);
}

pub(super) fn malformed_jsonl_line() {
    let families = Families::batch();
    let malformed = FileData::Direct(FileBody::Bytes(b"{\"custom_id\":".to_vec()));
    let (json_result, _) = run_upload(
        &families,
        &malformed,
        Some(CreatePurpose::Batch),
        "",
        &mut booking_params(),
    );
    assert!(matches!(
        json_result.expect_err("the malformed line is refused"),
        BatchRecordsError::Json(_)
    ));
    let invalid_utf8 = FileData::Direct(FileBody::Bytes(vec![0xff, 0xfe]));
    let (utf8_result, _) = run_upload(
        &families,
        &invalid_utf8,
        Some(CreatePurpose::Batch),
        "",
        &mut booking_params(),
    );
    assert!(matches!(
        utf8_result.expect_err("the invalid UTF-8 line is refused"),
        BatchRecordsError::Utf8(_)
    ));
    // a record whose body is not an object never reaches a family
    let family = FileData::Direct(FileBody::Bytes(
        b"{\"custom_id\":\"a\",\"body\":[1]}".to_vec(),
    ));
    let (shape_result, _) = run_upload(
        &Families::batch(),
        &family,
        Some(CreatePurpose::Batch),
        "",
        &mut booking_params(),
    );
    assert!(matches!(
        shape_result.expect_err("the array body is refused"),
        BatchRecordsError::RecordShape
    ));
    // a non-batch file keeps its bytes as UTF-8 text
    let plain = FileData::Tuple3 {
        filename: None,
        content: FileBody::Bytes(b"plain text".to_vec()),
        content_type: Some(String::from("text/plain")),
    };
    let (plain_result, _) = run_upload(
        &Families::default(),
        &plain,
        None,
        "",
        &mut booking_params(),
    );
    let plain_handoff = plain_result.expect("the plain upload signs");
    assert_eq!(plain_handoff.data, "plain text");
    let bare = FileData::Direct(FileBody::BareString(String::from("plain text")));
    let (bare_result, _) = run_upload(&Families::default(), &bare, None, "", &mut booking_params());
    assert!(matches!(
        bare_result.expect_err("the bare string is refused"),
        BatchRecordsError::Media(_)
    ));
    let (missing, _) = run_upload_missing_file(&Families::default());
    assert_eq!(missing.to_string(), "file is required");
}

pub(super) fn nova_batch_record() {
    let families = nova_families();
    let records = transform(
        &families,
        r#"{"custom_id":"n","url":"/v1/chat/completions","body":{"model":"amazon.nova-pro-v1:0","messages":[{"role":"developer","content":"be nice"},{"content":"hi","role":"user"}],"max_tokens":16,"top_k":5}}"#,
        "",
    )
    .expect("the nova record transforms");
    assert_eq!(
        value(&records[0]),
        serde_json::json!({"recordId": "n", "modelInput": {"messages": [], "inferenceConfig": {}}})
    );
    let calls = families.nova_calls();
    assert_eq!(calls[0].model, "amazon.nova-pro-v1:0");
    // the nova branch hands every non-message field to the mapper, with the developer message projected
    assert_eq!(
        value(&Json::List(calls[0].messages.clone())),
        serde_json::json!([
            {"role": "system", "content": "be nice"},
            {"content": "hi", "role": "user"}
        ])
    );
    assert_eq!(
        calls[0].optional_params,
        params(r#"{"max_tokens":16,"top_k":5}"#)
    );
    assert!(families.anthropic_params_calls().is_empty());
}

fn nova_families() -> Families {
    let mut families = Families::default();
    families.nova_body = Some(json(r#"{"messages":[],"inferenceConfig":{}}"#));
    families
}

pub(super) fn upload_signing_handoff() {
    let families = Families::batch();
    let file = batch_file();
    let mut litellm_params = booking_params();
    litellm_params.insert(
        String::from("s3_encryption_key_id"),
        RequestValue::Text(String::from("key-1")),
    );
    let (booking, after) = run_upload(
        &families,
        &file,
        Some(CreatePurpose::Batch),
        "",
        &mut litellm_params,
    );
    let signed = booking.expect("the batch upload signs");
    assert_eq!(signed.method, "PUT");
    assert!(
        signed
            .url
            .starts_with("https://s3.us-east-1.amazonaws.com/my-bucket/")
    );
    assert!(
        signed
            .url
            .ends_with(&format!("-{}.jsonl", FIXED_UUID.hyphenated()))
    );
    let headers = &signed.headers;
    assert!(headers.contains(&(
        String::from("Content-Type"),
        String::from("application/json")
    )));
    assert!(headers.contains(&(String::from("Content-Language"), String::from("en"))));
    assert!(headers.contains(&(
        String::from("Cache-Control"),
        String::from("private, immutable, max-age=31536000, s-maxage=0")
    )));
    assert!(headers.contains(&(
        String::from("x-amz-server-side-encryption"),
        String::from("aws:kms")
    )));
    assert!(headers.contains(&(
        String::from("x-amz-server-side-encryption-aws-kms-key-id"),
        String::from("key-1")
    )));
    assert!(headers.contains(&(
        String::from("Content-Length"),
        signed.data.len().to_string()
    )));
    let authorization = headers
        .iter()
        .find(|(name, _value)| name == "Authorization")
        .expect("the upload is signed")
        .1
        .clone();
    assert!(authorization.contains(ACCESS_KEY));
    // the signature names the s3_region_name region, never the default AWS region
    assert!(authorization.contains("/us-east-1/s3/aws4_request"));
    assert!(!authorization.contains(SECRET_KEY));
    assert!(headers.iter().any(|(name, _value)| name == "X-Amz-Date"));
    assert!(
        headers
            .iter()
            .any(|(name, _value)| name == "X-Amz-Content-SHA256")
    );
    assert_eq!(
        after.get("upload_url"),
        Some(&RequestValue::Text(signed.url.clone()))
    );
    assert_eq!(
        after.get("_s3_upload_content_length"),
        Some(&RequestValue::Raw(signed.data.len().to_string()))
    );
    // the same batch record with a target model instead of a record model takes the litellm model
    let mut params_with_model = booking_params();
    params_with_model.insert(
        String::from("model"),
        RequestValue::Text(String::from(GATEWAY_MODEL)),
    );
    let (target_upload, _after) = run_upload(
        &Families::batch(),
        &file,
        Some(CreatePurpose::Batch),
        "",
        &mut params_with_model,
    );
    assert!(
        target_upload
            .expect("the target model upload signs")
            .data
            .contains("\"messages\"")
    );
    // an empty s3_region_name counts as unset, so the signature falls back to the AWS default region
    let mut empty_region = booking_params();
    empty_region.insert(
        String::from("s3_region_name"),
        RequestValue::Text(String::new()),
    );
    let (default_region_result, _after) = run_upload(
        &Families::batch(),
        &file,
        Some(CreatePurpose::Batch),
        "",
        &mut empty_region,
    );
    let default_region = default_region_result.expect("the default region upload signs");
    let default_region_authorization = default_region
        .headers
        .iter()
        .find(|(name, _value)| name == "Authorization")
        .expect("the default region upload is signed")
        .1
        .clone();
    assert!(default_region_authorization.contains("/us-west-2/s3/aws4_request"));
    // a record model no provider knows takes the litellm model when that one names a Converse provider
    let unknown = FileData::Tuple3 {
        filename: None,
        content: FileBody::Bytes(
            b"{\"custom_id\":\"a\",\"body\":{\"model\":\"my-custom\",\"messages\":[{\"content\":\"hi\",\"role\":\"user\"}]}}"
                .to_vec(),
        ),
        content_type: Some(String::from("application/jsonl")),
    };
    let mut converse = booking_params();
    converse.insert(
        String::from("model"),
        RequestValue::Text(String::from("amazon.nova-pro-v1:0")),
    );
    let (targeted_result, _after) = run_upload(
        &Families::batch(),
        &unknown,
        Some(CreatePurpose::Batch),
        "",
        &mut converse,
    );
    let targeted = targeted_result.expect("the target model upload signs");
    assert!(targeted.data.contains("inferenceConfig"));
}
