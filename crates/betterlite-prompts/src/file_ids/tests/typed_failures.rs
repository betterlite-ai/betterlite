use crate::file_ids::mapping::FileIdMapping;
use crate::file_ids::responses::ResponsesInput;
use crate::file_ids::tools::ResponsesTool;
use crate::message_content::Message;
use rstest::rstest;

#[derive(Clone, Copy)]
enum Projection {
    Messages,
    ResponsesInput,
    Tools,
    Mapping,
}

fn decode(kind: Projection, text: &str) -> Result<(), serde_json::Error> {
    match kind {
        Projection::Messages => serde_json::from_str::<Vec<Message>>(text).map(drop),
        Projection::ResponsesInput => serde_json::from_str::<ResponsesInput>(text).map(drop),
        Projection::Tools => serde_json::from_str::<Option<Vec<ResponsesTool>>>(text).map(drop),
        Projection::Mapping => serde_json::from_str::<FileIdMapping>(text).map(drop),
    }
}

#[rstest]
#[case::content_part_without_a_type(
    Projection::Messages,
    r#"[{"role":"user","content":[{"file":{"file_id":"file-a"}}]}]"#,
    false
)]
#[case::non_object_content_parts(
    Projection::Messages,
    r#"[{"role":"user","content":[1,2]}]"#,
    false
)]
#[case::empty_message_list(Projection::Messages, "[]", true)]
#[case::responses_input_object(Projection::ResponsesInput, r#"{"role":"user"}"#, false)]
#[case::tools_string(Projection::Tools, r#""nope""#, false)]
#[case::absent_tools(Projection::Tools, "null", true)]
#[case::mapping_value_is_not_a_string(
    Projection::Mapping,
    r#"{"file-a":{"deployment-1":5}}"#,
    false
)]
#[case::mapping_entry_is_not_an_object(
    Projection::Mapping,
    r#"{"file-a":["deployment-1"]}"#,
    false
)]
fn model_file_ids_behavior_tests_behavior(
    #[case] kind: Projection,
    #[case] input: &str,
    #[case] expected_ok: bool,
) {
    assert_eq!(decode(kind, input).is_ok(), expected_ok);
}
