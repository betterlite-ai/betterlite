use crate::realtime_output::events::ModelFields;
use crate::realtime_output::metadata::{decode_generation_metadata, parse_generation_stage};

fn assert_stage(fields: &str, expected: Option<&str>) {
    let stage = decode_generation_metadata(fields).expect("readable metadata");
    assert_eq!(stage.as_deref(), expected, "{fields}");
    let carried = ModelFields::Text(fields.to_owned());
    assert_eq!(
        parse_generation_stage(Some(&carried)).as_deref(),
        expected,
        "{fields}"
    );
}

fn assert_unreadable(fields: &str) {
    assert!(decode_generation_metadata(fields).is_err(), "{fields}");
    let carried = ModelFields::Text(fields.to_owned());
    assert_eq!(parse_generation_stage(Some(&carried)), None, "{fields}");
}

pub(super) fn generation_stage() {
    assert_stage(r#"{"generationStage":"FINAL"}"#, Some("FINAL"));
    assert_stage(r#"{"generationStage":"SPECULATIVE"}"#, Some("SPECULATIVE"));
    assert_stage(r#"{"generationStage":""}"#, Some(""));
    assert_stage(r#"{"generationStage":"final"}"#, Some("final"));
    assert_stage(
        r#"{"generationStage":"OTHER","extra":[1,{"a":null}]}"#,
        Some("OTHER"),
    );
    assert_stage(
        r#"{"generationStage":"A","generationStage":"B"}"#,
        Some("B"),
    );
    assert_stage(r#"{"generationStage":null}"#, None);
    assert_stage(r#"{"generationStage":5}"#, None);
    assert_stage(r#"{"generationStage":["FINAL"]}"#, None);
    assert_stage(r#"{"generationStage":{"stage":"FINAL"}}"#, None);
    assert_stage("{}", None);
    assert_stage(r#"["FINAL"]"#, None);
    assert_stage(r#""FINAL""#, None);
    assert_stage("5", None);
    assert_stage("null", None);
    assert_unreadable("");
    assert_unreadable("{");
    assert_unreadable(r#"{"generationStage":"FINAL""#);
    assert_unreadable("not json");
    assert_unreadable(r#"{"generationStage":"FINAL"} trailing"#);
    non_string_fields_carry_no_stage();
}

fn non_string_fields_carry_no_stage() {
    assert_eq!(parse_generation_stage(None), None);
    let number: ModelFields = serde_json::from_str("5").expect("a non-string field");
    assert_eq!(parse_generation_stage(Some(&number)), None);
    let object: ModelFields =
        serde_json::from_str(r#"{"generationStage":"FINAL"}"#).expect("an object field");
    assert_eq!(parse_generation_stage(Some(&object)), None);
}
