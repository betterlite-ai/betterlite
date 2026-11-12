use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::input;
use super::support::json;
use super::support::object;
use super::support::rejection;
use super::support::resolved_files;
use super::support::serialized;
use crate::gemini_embedding_records::error::GeminiEmbeddingError;
use crate::gemini_embedding_records::input::EmbeddingInput;
use crate::gemini_embedding_records::input::InputEntry;
use crate::gemini_embedding_records::request::transform_batch_request;
use crate::gemini_embedding_records::request::transform_embed_content;

const MODEL: &str = "gemini-embedding-001";

fn batch(source: &str, optional: &str, resolved: &str) -> Result<Json, GeminiEmbeddingError> {
    transform_batch_request(
        &input(source),
        MODEL,
        &object(optional),
        &resolved_files(resolved),
    )
    .map(|request| serialized(&request))
}

fn embed_content(
    source: &str,
    optional: &str,
    resolved: &str,
) -> Result<Json, GeminiEmbeddingError> {
    let request =
        transform_embed_content(&input(source), &object(optional), &resolved_files(resolved));
    request.map(|built| serialized(&built))
}

#[rstest]
#[case::text(
    r#""hello""#,
    r#"{"requests":[{"model":"models/gemini-embedding-001","content":{"parts":[{"text":"hello"}]}}]}"#
)]
#[case::texts(
    r#"["a","b"]"#,
    r#"{"requests":[
        {"model":"models/gemini-embedding-001","content":{"parts":[{"text":"a"}]}},
        {"model":"models/gemini-embedding-001","content":{"parts":[{"text":"b"}]}}
    ]}"#
)]
#[case::combined_group(
    r#"[["a","b"]]"#,
    r#"{"requests":[{"model":"models/gemini-embedding-001","content":{"parts":[{"text":"a"},{"text":"b"}]}}]}"#
)]
#[case::mixed_group_and_text(
    r#"["a",["b","c"],"d"]"#,
    r#"{"requests":[
        {"model":"models/gemini-embedding-001","content":{"parts":[{"text":"a"}]}},
        {"model":"models/gemini-embedding-001","content":{"parts":[{"text":"b"},{"text":"c"}]}},
        {"model":"models/gemini-embedding-001","content":{"parts":[{"text":"d"}]}}
    ]}"#
)]
#[case::empty_list("[]", r#"{"requests":[]}"#)]
fn batch_requests_follow_the_input_grouping(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(batch(source, "{}", "{}").unwrap(), json(expected));
}

#[rstest]
#[case::inline_data(
    r#"["data:image/png;base64,AAAA"]"#,
    "{}",
    r#"{"requests":[{"model":"models/gemini-embedding-001","content":{"parts":[
        {"inline_data":{"mime_type":"image/png","data":"AAAA"}}
    ]}}]}"#
)]
#[case::gcs_file_data(
    r#"["gs://bucket/clip.MOV"]"#,
    "{}",
    r#"{"requests":[{"model":"models/gemini-embedding-001","content":{"parts":[
        {"file_data":{"mime_type":"video/quicktime","file_uri":"gs://bucket/clip.MOV"}}
    ]}}]}"#
)]
#[case::resolved_file(
    r#"["files/abc"]"#,
    r#"{"files/abc":{"mime_type":"application/pdf","uri":"https://example.com/v1beta/files/abc"}}"#,
    r#"{"requests":[{"model":"models/gemini-embedding-001","content":{"parts":[
        {"file_data":{"mime_type":"application/pdf","file_uri":"https://example.com/v1beta/files/abc"}}
    ]}}]}"#
)]
#[case::http_url_stays_text(
    r#"["https://example.com/photo.png"]"#,
    "{}",
    r#"{"requests":[{"model":"models/gemini-embedding-001","content":{"parts":[
        {"text":"https://example.com/photo.png"}
    ]}}]}"#
)]
#[case::data_url_without_base64_stays_text(
    r#"["data:image/png,AAAA"]"#,
    "{}",
    r#"{"requests":[{"model":"models/gemini-embedding-001","content":{"parts":[
        {"text":"data:image/png,AAAA"}
    ]}}]}"#
)]
fn batch_parts_classify_media(
    #[case] source: &str,
    #[case] resolved: &str,
    #[case] expected: &str,
) {
    assert_eq!(batch(source, "{}", resolved).unwrap(), json(expected));
}

#[rstest]
#[case::aliases_overwrite_natives(
    r#"{"dimensions":8,"outputDimensionality":4,"task_type":"RETRIEVAL_DOCUMENT","taskType":"SEMANTIC_SIMILARITY","title":"doc","user":"me","encoding_format":"float"}"#,
    r#"{"model":"models/gemini-embedding-001","content":{"parts":[{"text":"hello"}]},"outputDimensionality":8,"taskType":"RETRIEVAL_DOCUMENT","title":"doc"}"#
)]
#[case::native_spellings(
    r#"{"outputDimensionality":4,"taskType":"CLUSTERING","title":"t"}"#,
    r#"{"model":"models/gemini-embedding-001","content":{"parts":[{"text":"hello"}]},"outputDimensionality":4,"taskType":"CLUSTERING","title":"t"}"#
)]
#[case::alias_spellings(
    r#"{"dimensions":2,"task_type":"FACT_VERIFICATION"}"#,
    r#"{"model":"models/gemini-embedding-001","content":{"parts":[{"text":"hello"}]},"outputDimensionality":2,"taskType":"FACT_VERIFICATION"}"#
)]
#[case::nulls_take_the_default(
    r#"{"dimensions":null,"taskType":"CLASSIFICATION","title":null}"#,
    r#"{"model":"models/gemini-embedding-001","content":{"parts":[{"text":"hello"}]},"taskType":"CLASSIFICATION"}"#
)]
#[case::null_alias_clears_the_native(
    r#"{"dimensions":null,"outputDimensionality":4}"#,
    r#"{"model":"models/gemini-embedding-001","content":{"parts":[{"text":"hello"}]}}"#
)]
#[case::unknown_parameters_drop(
    r#"{"auto_truncate":true,"user":"u","encoding_format":"float"}"#,
    r#"{"model":"models/gemini-embedding-001","content":{"parts":[{"text":"hello"}]}}"#
)]
fn batch_parameters_filter_to_embedding_fields(#[case] optional: &str, #[case] expected: &str) {
    let value = batch(r#""hello""#, optional, "{}").unwrap();
    let Json::Object(fields) = value else {
        panic!("a batch request must be an object");
    };
    let Some(Json::List(requests)) = fields.get("requests") else {
        panic!("a batch request must hold a request list");
    };
    assert_eq!(requests.len(), 1);
    assert_eq!(requests.first(), Some(&json(expected)));
}

#[rstest]
#[case::empty_group("[[]]")]
#[case::empty_group_after_text(r#"["a",[]]"#)]
fn batch_rejects_empty_groups(#[case] source: &str) {
    let error = batch(source, "{}", "{}").unwrap_err();
    assert!(rejection(error).contains("combined embedding group"));
}

#[rstest]
#[case::text(r#""files/missing""#)]
#[case::in_group(r#"[["files/missing"]]"#)]
fn batch_rejects_unresolved_file_references(#[case] source: &str) {
    let error = batch(source, "{}", "{}").unwrap_err();
    assert!(rejection(error).contains("files/missing"));
}

#[rstest]
#[case::unsupported_extension(r#""gs://bucket/photo.gif""#)]
#[case::extension_with_query(r#""gs://bucket/photo.png?x=1""#)]
fn batch_rejects_unsupported_gcs_extensions(#[case] source: &str) {
    let error = batch(source, "{}", "{}").unwrap_err();
    assert!(rejection(error).contains("no supported embedding media type matches"));
}

#[rstest]
#[case::unsupported_mime(r#""data:text/plain;base64,AAAA""#)]
#[case::uppercase_mime(r#""data:IMAGE/PNG;base64,AAAA""#)]
fn batch_rejects_unsupported_data_url_media(#[case] source: &str) {
    let error = batch(source, "{}", "{}").unwrap_err();
    assert!(rejection(error).contains("is not a supported embedding media type"));
}

#[rstest]
#[case::text_dimensions(r#"{"dimensions":"8"}"#)]
#[case::fractional_dimensions(r#"{"dimensions":8.5}"#)]
fn batch_rejects_non_integer_dimensions(#[case] optional: &str) {
    let error = batch(r#""hello""#, optional, "{}").unwrap_err();
    assert!(rejection(error).contains("outputDimensionality value must be a whole number"));
}

#[rstest]
#[case::unknown_task_type(r#"{"task_type":"BOGUS"}"#)]
#[case::unknown_native_task_type(r#"{"taskType":"retrieval_query"}"#)]
fn batch_rejects_unknown_task_types(#[case] optional: &str) {
    let error = batch(r#""hello""#, optional, "{}").unwrap_err();
    assert!(rejection(error).contains("is not a Gemini embedding task type"));
}

#[rstest]
#[case::title(r#"{"title":5}"#)]
#[case::task_type(r#"{"taskType":5}"#)]
fn batch_rejects_mistyped_text_parameters(#[case] optional: &str) {
    let error = batch(r#""hello""#, optional, "{}").unwrap_err();
    assert!(rejection(error).contains("must be text"));
}

#[rstest]
#[case::text(r#""hello""#, r#"{"content":{"parts":[{"text":"hello"}]}}"#)]
#[case::texts(r#"["a","b"]"#, r#"{"content":{"parts":[{"text":"a"},{"text":"b"}]}}"#)]
#[case::empty("[]", r#"{"content":{"parts":[]}}"#)]
#[case::media(
    r#"["data:image/jpeg;base64,AA","gs://bucket/a.pdf"]"#,
    r#"{"content":{"parts":[
        {"inline_data":{"mime_type":"image/jpeg","data":"AA"}},
        {"file_data":{"mime_type":"application/pdf","file_uri":"gs://bucket/a.pdf"}}
    ]}}"#
)]
fn embed_content_combines_a_flat_input(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(embed_content(source, "{}", "{}").unwrap(), json(expected));
}

#[test]
fn embed_content_carries_filtered_parameters() {
    let value = embed_content(
        r#""hello""#,
        r#"{"dimensions":3,"task_type":"QUESTION_ANSWERING","title":"t","user":"u"}"#,
        "{}",
    )
    .unwrap();
    assert_eq!(
        value,
        json(
            r#"{"content":{"parts":[{"text":"hello"}]},"outputDimensionality":3,"taskType":"QUESTION_ANSWERING","title":"t"}"#
        )
    );
}

#[rstest]
#[case::group(r#"[["a","b"]]"#)]
#[case::group_after_text(r#"["a",["b"]]"#)]
fn embed_content_rejects_nested_groups(#[case] source: &str) {
    let error = embed_content(source, "{}", "{}").unwrap_err();
    assert!(rejection(error).contains("combined embedding groups"));
}

#[rstest]
#[case::text(r#""hello""#, false)]
#[case::texts(r#"["a","b"]"#, false)]
#[case::group(r#"[["a","b"]]"#, true)]
#[case::mixed(r#"["a",["b"]]"#, true)]
#[case::empty("[]", false)]
fn input_decodes_the_declared_shapes(#[case] source: &str, #[case] nested: bool) {
    let decoded: EmbeddingInput = serde_json::from_str(source).unwrap();
    let found = matches!(
        &decoded,
        EmbeddingInput::Entries(entries)
            if entries.iter().any(|entry| matches!(entry, InputEntry::Group(_)))
    );
    assert_eq!(found, nested);
}

#[rstest]
#[case::number("[1]")]
#[case::booleans("[true]")]
#[case::numbers_as_group("[[1,2]]")]
#[case::object(r#"{"input":"a"}"#)]
fn input_rejects_values_outside_the_declared_shapes(#[case] source: &str) {
    serde_json::from_str::<EmbeddingInput>(source).unwrap_err();
}
