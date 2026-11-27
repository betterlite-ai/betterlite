#![cfg(test)]

use rstest::rstest;

use super::McpToolResult;

fn decode(document: &str) -> Result<McpToolResult, serde_path_to_error::Error<serde_json::Error>> {
    serde_path_to_error::deserialize(&mut serde_json::Deserializer::from_str(document))
}

#[rstest]
#[case::id_and_name_set(
    Some("call_1"),
    "done",
    Some("search"),
    r#"{"tool_call_id":"call_1","result":"done","name":"search"}"#
)]
#[case::id_and_name_null(
    None,
    "done",
    None,
    r#"{"tool_call_id":null,"result":"done","name":null}"#
)]
fn serializes_keys_in_order(
    #[case] tool_call_id: Option<&str>,
    #[case] result: &str,
    #[case] name: Option<&str>,
    #[case] expected: &str,
) {
    let record = McpToolResult {
        tool_call_id: tool_call_id.map(str::to_owned),
        result: result.to_owned(),
        name: name.map(str::to_owned),
    };
    let encoded = serde_json::to_string(&record).expect("the record encodes");
    assert_eq!(encoded, expected);
}

#[rstest]
#[case::missing_tool_call_id(r#"{"result":"done","name":null}"#, Err("tool_call_id"))]
#[case::missing_result(r#"{"tool_call_id":null,"name":null}"#, Err("result"))]
#[case::missing_name(r#"{"tool_call_id":null,"result":"done"}"#, Err("name"))]
#[case::null_result(r#"{"tool_call_id":null,"result":null,"name":null}"#, Err("result"))]
#[case::numeric_tool_call_id(
    r#"{"tool_call_id":4,"result":"done","name":null}"#,
    Err("tool_call_id")
)]
#[case::numeric_name(r#"{"tool_call_id":null,"result":"done","name":7}"#, Err("name"))]
#[case::null_id_and_name(
    r#"{"tool_call_id":null,"result":"done","name":null}"#,
    Ok(McpToolResult {
        tool_call_id: None,
        result: "done".to_owned(),
        name: None,
    })
)]
#[case::full_object(
    r#"{"tool_call_id":"call_1","result":"done","name":"search"}"#,
    Ok(McpToolResult {
        tool_call_id: Some("call_1".to_owned()),
        result: "done".to_owned(),
        name: Some("search".to_owned()),
    })
)]
fn decode_requires_every_key(
    #[case] document: &str,
    #[case] expected: Result<McpToolResult, &str>,
) {
    match expected {
        Ok(record) => {
            let decoded = decode(document).expect("the full record decodes");
            assert_eq!(decoded, record);
        }
        Err(field) => {
            let error = decode(document).expect_err("an incomplete or mistyped record is refused");
            assert!(
                error.to_string().contains(field),
                "the error names {field}: {error}"
            );
        }
    }
}
