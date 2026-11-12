#![cfg(test)]

use std::path::PathBuf;
use std::str::from_utf8;

use betterlite_error::proxy_exception::ProxyException;
use betterlite_openai::api::batch_records::BatchRecordsError as WireError;
use betterlite_openai::files::records::FilesPurpose;
use rstest::rstest;

use super::{
    BatchFileContent, BatchRecords, BatchRecordsError, BatchRecordsInput, BatchRecordsResult,
    ReplacedContent, ReplacedFile,
};
use crate::api::api_boundaries::ApiLimits;
use crate::routes::CallType;

fn service() -> BatchRecords {
    BatchRecords {
        limits: ApiLimits::default(),
    }
}

fn parse(content: &[u8]) -> Result<BatchRecordsResult, BatchRecordsError> {
    service().run(BatchRecordsInput::ParseJsonl {
        content: content.to_vec(),
    })
}

fn parsed_rows(content: &str) -> Vec<String> {
    let Ok(BatchRecordsResult::Rows(rows)) = parse(content.as_bytes()) else {
        panic!("expected rows for {content:?}");
    };
    rows.iter()
        .map(|row| serde_json::to_string(row).unwrap())
        .collect()
}

fn replace(content: &[u8], new_model_name: &str) -> ReplacedFile {
    let input = BatchRecordsInput::ReplaceModel {
        file: BatchFileContent::Bytes(content.to_vec()),
        new_model_name: new_model_name.to_owned(),
    };
    let Ok(BatchRecordsResult::Replaced(file)) = service().run(input) else {
        panic!("expected a replaced file for {content:?}");
    };
    file
}

fn rewritten_lines(content: &str) -> Vec<String> {
    let file = replace(content.as_bytes(), "deployment");
    assert_eq!(file.logged_preview, None);
    let ReplacedContent::Rewritten(output) = file.content else {
        panic!("expected a rewritten file for {content:?}");
    };
    assert_eq!(output.name, "modified_file.jsonl");
    assert_eq!(output.content_type, "application/jsonl");
    from_utf8(&output.content)
        .unwrap()
        .split('\n')
        .map(str::to_owned)
        .collect()
}

fn limit_error(limit: i64) -> Box<ProxyException> {
    let input = BatchRecordsInput::ValidateListLimit { limit: Some(limit) };
    let Err(BatchRecordsError::ListLimit(exception)) = service().run(input) else {
        panic!("expected a list limit error for {limit}");
    };
    exception
}

#[rstest]
#[case::object_spans_lines(
    "{\n  \"id\": 1,\n  \"msg\": \"a\"\n}\n{\"id\":2}\n",
    &["{\"id\":1,\"msg\":\"a\"}", "{\"id\":2}"]
)]
#[case::escaped_newline_in_a_string(
    "{\"id\":1,\"msg\":\"Line 1\\nLine 2\"}\n{\"id\":2,\"msg\":\"test\"}",
    &["{\"id\":1,\"msg\":\"Line 1\\nLine 2\"}", "{\"id\":2,\"msg\":\"test\"}"]
)]
#[case::blank_lines_between_rows("{\"id\":1}\n\n   \n{\"id\":2}\n", &["{\"id\":1}", "{\"id\":2}"])]
#[case::empty_content("", &[])]
fn jsonl_embedded_newline(#[case] content: &str, #[case] expected: &[&str]) {
    assert_eq!(parsed_rows(content), expected);
}

#[rstest]
#[case::truncated_last_row("{\"id\":1}\n{\"id\":2,\n", "{\"id\":2,\n")]
#[case::text_after_a_row("{\"id\":1} x\n", "{\"id\":1} x\n")]
#[case::two_rows_on_one_line("{\"id\":1}{\"id\":2}\n", "{\"id\":1}{\"id\":2}\n")]
fn jsonl_unparsed_tail_is_an_error(#[case] content: &str, #[case] preview: &str) {
    let Err(BatchRecordsError::Unparsed(tail)) = parse(content.as_bytes()) else {
        panic!("expected an unparsed tail for {content:?}");
    };
    assert_eq!(tail.preview, preview);
}

#[rstest]
#[case::replaces_an_existing_model(
    "{\"custom_id\":\"a\",\"body\":{\"model\":\"alias\",\"input\":\"x\"}}\n",
    &["{\"custom_id\":\"a\",\"body\":{\"input\":\"x\",\"model\":\"deployment\"}}"]
)]
#[case::adds_a_missing_model(
    "{\"custom_id\":\"a\",\"body\":{\"input\":\"x\"}}",
    &["{\"custom_id\":\"a\",\"body\":{\"input\":\"x\",\"model\":\"deployment\"}}"]
)]
#[case::joins_rows_with_one_newline(
    "{\"body\":{\"n\":1}}\n\n{\"body\":{\"n\":2}}\n",
    &["{\"body\":{\"model\":\"deployment\",\"n\":1}}", "{\"body\":{\"model\":\"deployment\",\"n\":2}}"]
)]
#[case::row_spanning_lines(
    "{\"custom_id\":\"a\",\n \"body\":{\"n\":1}}\n",
    &["{\"custom_id\":\"a\",\"body\":{\"model\":\"deployment\",\"n\":1}}"]
)]
#[case::leaves_rows_without_an_object_body(
    "{\"body\":\"text\"}\n{\"custom_id\":\"a\"}\n[{\"body\":{}}]\n3\n",
    &["{\"body\":\"text\"}", "{\"custom_id\":\"a\"}", "[{\"body\":{}}]", "3"]
)]
fn replace_body_model(#[case] content: &str, #[case] expected: &[&str]) {
    assert_eq!(rewritten_lines(content), expected);
}

#[rstest]
#[case::string_id(
    "{\"custom_id\":\"request-1\",\"method\":\"POST\",\"url\":\"/v1/chat/completions\",\"body\":{\"model\":\"alias\"}}",
    "{\"custom_id\":\"request-1\",\"method\":\"POST\",\"url\":\"/v1/chat/completions\",\"body\":{\"model\":\"deployment\"}}"
)]
#[case::numeric_id(
    "{\"custom_id\":7,\"method\":\"POST\",\"url\":\"/v1/embeddings\",\"body\":{\"model\":\"alias\"}}",
    "{\"custom_id\":7,\"method\":\"POST\",\"url\":\"/v1/embeddings\",\"body\":{\"model\":\"deployment\"}}"
)]
#[case::empty_id(
    "{\"custom_id\":\"\",\"url\":\"/v1/chat/completions\",\"body\":{}}",
    "{\"custom_id\":\"\",\"url\":\"/v1/chat/completions\",\"body\":{\"model\":\"deployment\"}}"
)]
fn preserve_custom_id(#[case] row: &str, #[case] expected: &str) {
    assert_eq!(rewritten_lines(row), [expected]);
}

#[rstest]
#[case::batch_upload(FilesPurpose::Batch, false, true)]
#[case::batch_passthrough(FilesPurpose::Batch, true, false)]
#[case::assistants(FilesPurpose::Assistants, false, false)]
#[case::assistants_output(FilesPurpose::AssistantsOutput, false, false)]
#[case::batch_output(FilesPurpose::BatchOutput, false, false)]
#[case::fine_tune(FilesPurpose::FineTune, false, false)]
#[case::fine_tune_results(FilesPurpose::FineTuneResults, false, false)]
#[case::vision(FilesPurpose::Vision, false, false)]
#[case::user_data(FilesPurpose::UserData, false, false)]
#[case::evals(FilesPurpose::Evals, false, false)]
#[case::messages(FilesPurpose::Messages, false, false)]
#[case::vision_passthrough(FilesPurpose::Vision, true, false)]
fn native_model_no_replace(
    #[case] purpose: FilesPurpose,
    #[case] passthrough: bool,
    #[case] expected: bool,
) {
    let input = BatchRecordsInput::ShouldReplaceModel {
        purpose,
        passthrough,
    };
    assert_eq!(
        service().run(input).unwrap(),
        BatchRecordsResult::ShouldReplace(expected)
    );
}

#[rstest]
#[case::retrieve(CallType::RetrieveBatch, true)]
#[case::async_retrieve(CallType::AretrieveBatch, true)]
#[case::create(CallType::CreateBatch, false)]
#[case::async_create(CallType::AcreateBatch, false)]
#[case::cancel(CallType::CancelBatch, false)]
#[case::async_cancel(CallType::AcancelBatch, false)]
#[case::completion(CallType::Completion, false)]
fn batch_retrieve_call_type(#[case] call_type: CallType, #[case] expected: bool) {
    let input = BatchRecordsInput::IsBatchRetrieveCallType { call_type };
    assert_eq!(
        service().run(input).unwrap(),
        BatchRecordsResult::BatchRetrieve(expected)
    );
}

fn assert_limit_error(limit: i64, bound: &str, openai_code: &str) {
    let exception = limit_error(limit);
    assert_eq!(
        exception.message,
        format!("Invalid 'limit': integer {bound}, but got {limit} instead.")
    );
    assert_eq!(exception.error_type, "invalid_request_error");
    assert_eq!(exception.param.as_deref(), Some("limit"));
    assert_eq!(exception.code, "400");
    assert_eq!(exception.openai_code.as_deref(), Some(openai_code));
}

#[rstest]
#[case::one_below(-1)]
#[case::far_below(-5000)]
#[case::smallest(i64::MIN)]
fn batch_limit_below(#[case] limit: i64) {
    assert_limit_error(
        limit,
        "below minimum value. Expected a value >= 0",
        "integer_below_min_value",
    );
}

#[rstest]
#[case::one_above(101)]
#[case::far_above(5000)]
#[case::largest(i64::MAX)]
fn batch_limit_above(#[case] limit: i64) {
    assert_limit_error(
        limit,
        "above maximum value. Expected a value <= 100",
        "integer_above_max_value",
    );
}

#[rstest]
#[case::not_given(None)]
#[case::lowest(Some(0))]
#[case::one(Some(1))]
#[case::highest(Some(100))]
fn list_limit_accepts_the_range(#[case] limit: Option<i64>) {
    let input = BatchRecordsInput::ValidateListLimit { limit };
    assert_eq!(
        service().run(input).unwrap(),
        BatchRecordsResult::ListLimitAccepted
    );
}

#[rstest]
#[case::path(BatchFileContent::Path(PathBuf::from("batch.jsonl")), None)]
#[case::no_rows(BatchFileContent::Bytes(Vec::new()), None)]
#[case::only_blank_lines(BatchFileContent::Bytes(b"\n  \n".to_vec()), None)]
#[case::not_utf8(BatchFileContent::Bytes(b"{\"body\":\"\xff\"}\n".to_vec()), None)]
#[case::truncated_row(
    BatchFileContent::Bytes(b"{\"body\":{}}\n{\"body\":{".to_vec()),
    Some("{\"body\":{")
)]
#[case::text_after_a_row(
    BatchFileContent::Bytes(b"{\"body\":{}} x\n".to_vec()),
    Some("{\"body\":{}} x\n")
)]
fn replace_returns_the_original(
    #[case] file: BatchFileContent,
    #[case] logged_preview: Option<&str>,
) {
    let input = BatchRecordsInput::ReplaceModel {
        file,
        new_model_name: "deployment".to_owned(),
    };
    assert_eq!(
        service().run(input).unwrap(),
        BatchRecordsResult::Replaced(ReplacedFile {
            content: ReplacedContent::Original,
            logged_preview: logged_preview.map(str::to_owned),
        })
    );
}

#[rstest]
#[case::json_bytes(ApiLimits { max_json_bytes: 4, ..ApiLimits::default() })]
#[case::row_bytes(ApiLimits { max_event_bytes: 4, ..ApiLimits::default() })]
#[case::row_count(ApiLimits { max_items: 0, ..ApiLimits::default() })]
fn limits_fail_the_read(#[case] limits: ApiLimits) {
    let service = BatchRecords { limits };
    let parse = BatchRecordsInput::ParseJsonl {
        content: b"{\"body\":{}}\n".to_vec(),
    };
    assert!(matches!(
        service.run(parse),
        Err(BatchRecordsError::Decode(
            WireError::JsonTooLarge { .. }
                | WireError::RowTooLarge { .. }
                | WireError::TooManyRows { .. }
        ))
    ));
    let replace = BatchRecordsInput::ReplaceModel {
        file: BatchFileContent::Bytes(b"{\"body\":{}}\n".to_vec()),
        new_model_name: "deployment".to_owned(),
    };
    assert!(matches!(
        service.run(replace),
        Err(BatchRecordsError::Decode(_))
    ));
}

#[rstest]
#[case::batch_endpoint(Some("create_batch"), "litellm_metadata")]
#[case::private_create_batch(Some("_acreate_batch"), "litellm_metadata")]
#[case::generic_call(Some("generic_api_call"), "litellm_metadata")]
#[case::generic_call_with_fallbacks(Some("_ageneric_api_call_with_fallbacks"), "litellm_metadata")]
#[case::file(Some("acreate_file"), "litellm_metadata")]
#[case::chat(Some("acompletion"), "metadata")]
#[case::empty(Some(""), "metadata")]
#[case::absent(None, "metadata")]
fn router_metadata_name(#[case] function_name: Option<&str>, #[case] expected: &'static str) {
    let input = BatchRecordsInput::RouterMetadataName {
        function_name: function_name.map(str::to_owned),
    };
    assert_eq!(
        service().run(input).unwrap(),
        BatchRecordsResult::MetadataName(expected)
    );
}
