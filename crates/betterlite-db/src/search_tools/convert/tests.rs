#![cfg(test)]

use chrono::DateTime;
use rstest::rstest;

use crate::mcp_stored_contracts::JsonValue;
use crate::search_tools::convert::tool::SearchTool;
use crate::search_tools::convert::{SearchToolRow, isoformat};
use crate::search_tools::registry::SearchToolRegistry;
use crate::search_tools::tests::fakes::{PARAMS_TEXT, instant, stored_record, tool};

fn row(litellm_params: JsonValue, search_tool_info: Option<JsonValue>) -> SearchToolRow {
    SearchToolRow {
        search_tool_id: "tool-1".to_owned(),
        search_tool_name: "web".to_owned(),
        litellm_params,
        search_tool_info,
        created_at: "2026-02-25T06:13:20+00:00".to_owned(),
        updated_at: "2026-03-04T05:06:07+00:00".to_owned(),
    }
}

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::whole_second(1_772_600_767_000_000, "2026-03-04T05:06:07+00:00")]
#[case::millisecond_precision(1_772_600_767_123_000, "2026-03-04T05:06:07.123000+00:00")]
#[case::microsecond_precision(1_772_600_767_123_456, "2026-03-04T05:06:07.123456+00:00")]
#[case::one_microsecond(1_772_600_767_000_001, "2026-03-04T05:06:07.000001+00:00")]
fn convert_isoformat_matches_python(#[case] micros: i64, #[case] expected: &str) {
    let value = DateTime::from_timestamp_micros(micros).unwrap();
    assert_eq!(isoformat(value), expected);
}

#[test]
fn convert_record_turns_both_timestamps_into_iso_strings() {
    let converted = SearchToolRegistry::convert_prisma_to_dict(&stored_record("tool-1", "web"));
    assert_eq!(converted.search_tool_id, "tool-1");
    assert_eq!(converted.search_tool_name, "web");
    assert_eq!(
        converted.litellm_params,
        JsonValue::String(PARAMS_TEXT.to_owned())
    );
    assert_eq!(converted.created_at, "2026-02-25T06:13:20+00:00");
    assert_eq!(converted.updated_at, isoformat(instant()));
}

#[test]
fn convert_row_to_tool_reads_json_text_columns() {
    let parsed = SearchTool::try_from(row(
        JsonValue::String(r#"{"search_provider":"tavily","timeout":2.5}"#.to_owned()),
        Some(JsonValue::String(
            r#"{"description":"Web search"}"#.to_owned(),
        )),
    ))
    .unwrap();
    assert_eq!(parsed.search_tool_id.as_deref(), Some("tool-1"));
    assert_eq!(parsed.search_tool_name, "web");
    assert_eq!(parsed.litellm_params.timeout.unwrap().to_string(), "2.5");
    assert_eq!(parsed.search_tool_info.unwrap().len(), 1);
    assert_eq!(
        parsed.created_at.as_deref(),
        Some("2026-02-25T06:13:20+00:00")
    );
    assert_eq!(
        parsed.updated_at.as_deref(),
        Some("2026-03-04T05:06:07+00:00")
    );
}

#[test]
fn convert_row_to_tool_reads_json_object_columns() {
    let parsed = SearchTool::try_from(row(
        json(r#"{"search_provider":"exa","api_base":"https://exa.example"}"#),
        Some(json(r#"{"description":"x"}"#)),
    ))
    .unwrap();
    assert_eq!(parsed.litellm_params.search_provider, "exa");
    assert_eq!(
        parsed.litellm_params.api_base.as_deref(),
        Some("https://exa.example")
    );
    assert_eq!(parsed.search_tool_info.unwrap().len(), 1);
}

#[rstest]
#[case::absent(None)]
#[case::null(Some(JsonValue::Null))]
fn convert_row_to_tool_treats_missing_info_as_none(#[case] info: Option<JsonValue>) {
    let parsed = SearchTool::try_from(row(json(r#"{"search_provider":"exa"}"#), info)).unwrap();
    assert!(parsed.search_tool_info.is_none());
}

#[test]
fn convert_row_to_tool_keeps_an_empty_info_object() {
    let parsed = SearchTool::try_from(row(
        json(r#"{"search_provider":"exa"}"#),
        Some(JsonValue::String("{}".to_owned())),
    ))
    .unwrap();
    assert!(parsed.search_tool_info.unwrap().is_empty());
}

#[rstest]
#[case::unknown_key(
    json(r#"{"search_provider":"exa","surprise":1}"#),
    None,
    "litellm_params",
    "surprise"
)]
#[case::missing_provider(json(r#"{"api_base":"x"}"#), None, "litellm_params", "search_provider")]
#[case::wrong_type(
    json(r#"{"search_provider":"exa","max_retries":"many"}"#),
    None,
    "litellm_params",
    "expected i64"
)]
#[case::info_not_an_object(
    json(r#"{"search_provider":"exa"}"#),
    Some(json("[1]")),
    "search_tool_info",
    "invalid type"
)]
fn convert_row_to_tool_names_the_field_that_cannot_be_read(
    #[case] params: JsonValue,
    #[case] info: Option<JsonValue>,
    #[case] field: &str,
    #[case] detail: &str,
) {
    let message = SearchTool::try_from(row(params, info))
        .unwrap_err()
        .to_string();
    assert!(message.starts_with(&format!("search tool field {field} is not valid: ")));
    assert!(message.contains(detail));
}

#[test]
fn convert_tool_debug_output_hides_the_api_key() {
    let text = format!("{:?}", tool());
    assert!(text.contains("tavily"));
    assert!(!text.contains("sk-secret"));
}
