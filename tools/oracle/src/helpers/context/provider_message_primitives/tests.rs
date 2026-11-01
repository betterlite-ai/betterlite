#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, parse_helper_case};

#[test]
fn helper_seeds_select_provider_message_primitives() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_str().unwrap();
            name.starts_with("provider_message_primitives")
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    let selected = seeds.iter().all(|source| {
        parse_helper_case(source).unwrap();
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        case.call.operation == "provider_message_primitives"
    });
    assert!(selected);
}

#[rstest::rstest]
#[case::native("response-format-native")]
#[case::tool("response-format-tool")]
#[case::developer("developer-role-projection")]
#[case::iterator("iterator-initial-state")]
#[case::batch("batch-initializer")]
#[case::chat_id("chat-id-prefix")]
#[case::tool_call("tool-call-record-projection")]
fn provider_message_primitives_cases_match_litellm(#[case] name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/providers/provider-message-primitives")
        .join(format!("{name}.yaml"));
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let run = crate::helpers::run_case(&case, false).unwrap();
    assert!(matches!(run, crate::helpers::HelperRun::Match));
}

#[rstest::rstest]
#[case::number("{response_schema: 2}")]
#[case::zero("{response_schema: 0}")]
#[case::text("{response_schema: x}")]
#[case::list("{response_schema: [1]}")]
#[case::missing_schema("{json_schema: {name: answer}}")]
#[case::not_object("{json_schema: 5}")]
fn response_formats_match_litellm(#[case] value: &str) {
    matches_litellm(&format!(
        "function: response_format, optional_params: {{}}, value: {value}, supported: false, enforce_tool_choice: null"
    ));
}

#[test]
fn function_tool_calls_match_litellm() {
    matches_litellm(
        "function: tool_call, record: {id: c, type: function, function: {name: f}, custom: null}",
    );
}

fn matches_litellm(call: &str) {
    let source = format!(
        "kind: helper\nname: inline\ncall: {{provider_message_primitives: {{{call}}}}}\nenv: {{}}\ncompare: {{ignore: []}}\n"
    );
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    let run = crate::helpers::run_case(&case, false).unwrap();
    assert!(matches!(run, crate::helpers::HelperRun::Match));
}
