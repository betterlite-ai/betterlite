#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;

use crate::case::{HelperCase, helper::validate_helper};

#[test]
fn helper_seeds_select_xai_responses() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("xai_responses"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "xai_responses"
    }));
}

// the expected answers are LiteLLM's own, captured from the Python helper on the same case
#[rstest]
#[case::image_store_unchanged("image-store-unchanged.yaml", r#"[{"input":[{"type":"message","role":"user","content":[{"type":"input_image","image_url":"https://example.test/image.png"}]}],"store":false},{"input":[{"type":"message","role":"user","content":[{"type":"input_image","image_url":"https://example.test/image.png"}]}]},{"input":[],"store":true}]"#)]
#[case::legacy_key_and_oauth_base("legacy-key-and-oauth-base.yaml", r#"[{"Authorization":"Bearer explicit-key","X-Keep":"yes"},{"Authorization":"Bearer generic-key"},"https://api.x.ai/v1/responses","https://api.x.ai/v1/responses","https://explicit.example/v1/responses"]"#)]
#[case::missing_key("missing-key.yaml", r#"[{"raised":null},{"raised":401},"https://env.example/v1/responses","https://env.example/v1/responses"]"#)]
#[case::native_tools_and_container("native-tools-and-container.yaml", r#"[{"tools":[{"type":"code_interpreter"},{"type":"x_search","allowed_x_handles":["handle"],"excluded_x_handles":null,"from_date":"2026-01-01","to_date":"2026-02-01","enable_image_understanding":false,"enable_video_understanding":null},{"type":"function","name":"f","parameters":{"type":"object"}},"plain-string",7]}]"#)]
#[case::nested_filters_win("nested-filters-win.yaml", r#"[{"tools":[{"type":"web_search","filters":{"allowed_domains":["nested.example"],"excluded_domains":["gone.example"]},"enable_image_understanding":true},{"type":"web_search","filters":{"allowed_domains":null}},{"type":"web_search","filters":{"allowed_domains":["fallback.example"]}},{"type":"web_search"}]}]"#)]
#[case::no_native_websocket(
    "no-native-websocket.yaml",
    r#"[{"native_websocket":false},{"native_websocket":false}]"#
)]
#[case::oauth_token_and_base("oauth-token-and-base.yaml", r#"[{"Authorization":"Bearer oauth-token"},{"raised":null},"https://oauth.example/v1/responses","https://api.x.ai/v1/responses","https://explicit.example/v1/responses"]"#)]
#[case::single_tool_and_null_metadata("single-tool-and-null-metadata.yaml", r#"[{"store":true,"model":"grok-4","tools":[{"type":"web_search","filters":{"allowed_domains":["one.example"]}}]},{"tools":["plain"]},{"tools":[]},{"tools":0}]"#)]
#[case::terminal_cost_only("terminal-cost-only.yaml", r#"[{"cost":"1.25","ticks":12500000000},{"cost":"1.25","ticks":12500000000},{"cost":"0.25","ticks":-5},{"raised":200},{"cost":"0.5","ticks":5000000000,"class":"ResponseCompletedEvent"},{"cost":"0.25","ticks":2500000000,"class":"ResponseIncompleteEvent"},{"cost":null,"ticks":5000000000,"class":"ResponseCreatedEvent"},{"cost":null,"ticks":null,"class":"GenericEvent"}]"#)]
fn the_native_helper_answers_like_litellm(#[case] file: &str, #[case] litellm: &str) {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/xai-responses")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let answer = super::Op::decode_and_run(&case.call.input).unwrap();
    let expected: serde_json::Value = serde_json::from_str(litellm).unwrap();
    assert_eq!(answer, expected);
}
