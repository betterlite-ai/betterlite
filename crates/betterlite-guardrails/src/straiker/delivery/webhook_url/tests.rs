#![cfg(test)]

use rstest::rstest;
use secrecy::SecretString;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::straiker::delivery::fixtures::{
    Fakes, delivery, inputs, raw, reply, request_data, run, sent, settings,
};

const V1_URL: &str = "https://api.straiker.test/api/v1/detect/webhook";
const V3_URL: &str = "https://api.straiker.test/api/v3/detect";

#[rstest]
#[case::trailing_slash("https://api.straiker.test/", V1_URL)]
#[case::no_trailing_slash("https://api.straiker.test", V1_URL)]
#[case::several_slashes("https://api.straiker.test///", V1_URL)]
#[case::base_path(
    "https://host.test/gateway/",
    "https://host.test/gateway/api/v1/detect/webhook"
)]
fn the_v1_webhook_follows_the_api_base(#[case] base: &str, #[case] expected: &str) {
    let mut args = raw();
    args.api_base = base.to_owned();
    assert_eq!(settings(&args).webhook_url(), expected);
}

#[rstest]
#[case::unset(None)]
#[case::declared(Some("v3"))]
fn an_agent_key_posts_to_the_detect_path(#[case] configured: Option<&str>) {
    let mut args = raw();
    args.api_key = SecretString::from("sk_agt_live_key");
    args.api_version = configured.map(str::to_owned);
    assert_eq!(settings(&args).webhook_url(), V3_URL);
}

#[test]
fn a_declared_v3_version_uses_the_detect_path() {
    let mut args = raw();
    args.api_version = Some("v3".to_owned());
    assert_eq!(settings(&args).webhook_url(), V3_URL);
}

#[tokio::test]
async fn the_delivered_url_is_the_configured_one() {
    let fakes = Fakes::default();
    let resolved = settings(&raw());
    let handler = delivery(resolved, vec![reply(200, r#"{"action":"NONE"}"#)]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    assert_eq!(sent(&handler).first().unwrap().url, V1_URL);
}
