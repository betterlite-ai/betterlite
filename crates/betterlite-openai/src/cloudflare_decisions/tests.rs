#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;

use betterlite_models::decisions::ir;
use betterlite_models::decisions::{DecisionsJson, Extra};
use rstest::rstest;

use super::CloudflareDecisionsConfig;
use crate::decisions::transformation::provider_config::DecisionsProviderConfig as _;

const DEFAULT_BASE: &str = "https://api.cloudflare.com/client/v4/accounts/acct-1/ai/run";
const MISSING_BASE: &str =
    "Missing CLOUDFLARE_ACCOUNT_ID - set CLOUDFLARE_ACCOUNT_ID or pass api_base explicitly";

fn config(account_id: Option<&str>) -> CloudflareDecisionsConfig {
    CloudflareDecisionsConfig::new(account_id.map(str::to_owned))
}

#[rstest]
#[case::account_id(Some("acct-1"), Some(DEFAULT_BASE))]
#[case::trimmed_account_id(Some("  acct-1  "), Some(DEFAULT_BASE))]
#[case::unset_account_id(None, None)]
#[case::empty_account_id(Some(""), None)]
#[case::whitespace_account_id(Some("   "), None)]
fn default_base_comes_from_the_account_id(
    #[case] account_id: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        config(account_id).get_default_api_base().as_deref(),
        expected
    );
}

#[test]
fn environment_names_match_litellm() {
    assert_eq!(
        CloudflareDecisionsConfig::API_KEY_ENV,
        &["CLOUDFLARE_API_KEY"]
    );
    assert_eq!(
        CloudflareDecisionsConfig::API_BASE_ENV,
        &["CLOUDFLARE_API_BASE"]
    );
}

#[test]
fn missing_base_message_ignores_the_provider_name() {
    let config = config(None);
    assert_eq!(config.missing_api_base_message("cloudflare"), MISSING_BASE);
    assert_eq!(config.missing_api_base_message("other"), MISSING_BASE);
}

#[rstest]
#[case::canonical_left_alone("@cf/meta/llama", "@cf/meta/llama", "llama")]
#[case::bare_name("@cf/cloudflare/foo", "@cf/cloudflare/foo", "foo")]
#[case::provider_path("meta/llama", "@cf/cloudflare/meta/llama", "llama")]
#[case::bare_word("foo", "@cf/cloudflare/foo", "foo")]
#[case::empty_name("", "@cf/cloudflare/", "")]
#[case::trailing_slash("a/b/", "@cf/cloudflare/a/b/", "")]
#[case::many_segments("foo/bar/baz", "@cf/cloudflare/foo/bar/baz", "baz")]
fn model_names_match_litellm(#[case] model: &str, #[case] canonical: &str, #[case] request: &str) {
    let config = config(None);
    assert_eq!(config.canonical_model(model), canonical);
    assert_eq!(config.request_model(model), request);
}

#[rstest]
#[case::ai_run(
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run",
    "model",
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run/model"
)]
#[case::ai_run_with_slash(
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run/",
    "model",
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run/model"
)]
#[case::ai_run_with_two_slashes(
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run//",
    "model",
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run/model"
)]
#[case::ai_v1(
    "https://api.cloudflare.com/client/v4/accounts/A/ai/v1",
    "model",
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run/model"
)]
#[case::ai_v1_with_slash(
    "https://api.cloudflare.com/client/v4/accounts/A/ai/v1/",
    "model",
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run/model"
)]
#[case::ai_v1_with_two_slashes(
    "https://api.cloudflare.com/client/v4/accounts/A/ai/v1//",
    "model",
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run/model"
)]
#[case::other_base("https://proxy.example", "model", "https://proxy.example/ai/run/model")]
#[case::other_base_with_slash(
    "https://proxy.example/",
    "model",
    "https://proxy.example/ai/run/model"
)]
#[case::other_base_with_two_slashes(
    "https://proxy.example//",
    "model",
    "https://proxy.example/ai/run/model"
)]
#[case::ai_v1_not_at_the_end(
    "https://proxy.example/ai/v1/extra",
    "model",
    "https://proxy.example/ai/v1/extra/ai/run/model"
)]
#[case::ai_run_not_at_the_end(
    "https://proxy.example/ai/run/extra",
    "model",
    "https://proxy.example/ai/run/extra/ai/run/model"
)]
#[case::canonical_model(
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run",
    "@cf/cloudflare/foo",
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run/@cf/cloudflare/foo"
)]
fn complete_url_appends_the_model(
    #[case] api_base: &str,
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(config(None).get_complete_url(api_base, model), expected);
}

#[rstest]
#[case::answers_at_top(
    r#"{"answers":{"q":{"type":"noul","noul":1}},"result":{"answers":{}}}"#,
    r#"{"answers":{"q":{"type":"noul","noul":1}},"result":{"answers":{}}}"#
)]
#[case::answers_null_blocks_a_result(
    r#"{"answers":null,"result":{"answers":{}}}"#,
    r#"{"answers":null,"result":{"answers":{}}}"#
)]
#[case::result_object(
    r#"{"result":{"answers":{"q":{"type":"noul","noul":1}}},"id":"reply-1"}"#,
    r#"{"answers":{"q":{"type":"noul","noul":1}}}"#
)]
#[case::result_empty_object(r#"{"result":{},"id":1}"#, "{}")]
#[case::result_null(r#"{"result":null}"#, r#"{"result":null}"#)]
#[case::result_list(r#"{"result":[1,2,3]}"#, r#"{"result":[1,2,3]}"#)]
#[case::result_string(r#"{"result":"text"}"#, r#"{"result":"text"}"#)]
#[case::result_number(r#"{"result":7}"#, r#"{"result":7}"#)]
#[case::missing_result(r#"{"id":1}"#, r#"{"id":1}"#)]
#[case::top_level_array(r#"[{"result":{}}]"#, r#"[{"result":{}}]"#)]
#[case::top_level_string(r#""text""#, r#""text""#)]
#[case::top_level_null("null", "null")]
#[case::top_level_number("7", "7")]
#[case::not_json("not json", "not json")]
fn unwrap_response_follows_the_answers_and_result_rules(
    #[case] payload: &str,
    #[case] expected: &str,
) {
    let unwrapped = config(None).unwrap_response(payload.as_bytes());
    assert_eq!(from_utf8(unwrapped).unwrap(), expected);
}

#[test]
fn unwrap_response_returns_bytes_that_are_not_utf8_unchanged() {
    let payload = [0xff, 0xfe, b'{'];
    assert_eq!(config(None).unwrap_response(&payload), &payload[..]);
}

#[test]
fn parse_response_reads_a_wrapped_systemone_reply() {
    let request = ir::DecisionRequest {
        input: ir::DecisionInput::State(DecisionsJson::Text("state".to_owned())),
        questions: vec![ir::DecisionQuestion::Predicate(ir::PredicateQuestion {
            name: Some("reachable".to_owned()),
            instructions: Some(DecisionsJson::Text("is it reachable?".to_owned())),
            criteria: None,
            extra: Extra::default(),
        })],
        safety_identifier: None,
    };
    let payload = br#"{"result":{"model":"cf","answers":{"reachable":{"type":"noul","noul":1}}}}"#;
    let response = config(None).parse_response(payload, &request).unwrap();
    assert_eq!(response.answers.len(), 1);
    match &response.answers[0] {
        ir::DecisionAnswer::Predicate { probability, .. } => {
            assert_eq!(probability.as_i64(), Some(1));
        }
        other @ (ir::DecisionAnswer::Choice { .. }
        | ir::DecisionAnswer::Score { .. }
        | ir::DecisionAnswer::Refusal) => panic!("expected a predicate answer, got {other:?}"),
    }
}

#[rstest]
fn replays_the_cloudflare_decisions_reply_corpus(
    #[files("fuzz/corpus/cloudflare_decisions_reply/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    let _unwrapped = config(None).unwrap_response(&bytes);
}
