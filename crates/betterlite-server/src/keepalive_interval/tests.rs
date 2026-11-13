#![cfg(test)]

use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::{
    DeploymentLookup, PingSetting, agreed_keepalive_interval, coerce_keepalive_interval,
    ttft_interval_for, ttft_keepalive_interval,
};

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("valid JSON fixture")
}

fn parse_deployments(text: &str) -> Vec<JsonValue> {
    serde_json::from_str(text).expect("valid deployment list")
}

#[rstest]
#[case::none(None)]
#[case::not_a_number(Some(PingSetting::Text("later".to_owned())))]
#[case::empty_text(Some(PingSetting::Text(String::new())))]
#[case::infinity(Some(PingSetting::Text("inf".to_owned())))]
#[case::nan(Some(PingSetting::Seconds(f64::NAN)))]
#[case::infinite(Some(PingSetting::Seconds(f64::INFINITY)))]
#[case::zero(Some(PingSetting::Seconds(0.0)))]
#[case::zero_text(Some(PingSetting::Text("0".to_owned())))]
#[case::negative(Some(PingSetting::Seconds(-0.5)))]
#[case::negative_text(Some(PingSetting::Text("-1".to_owned())))]
#[case::beyond_duration(Some(PingSetting::Seconds(1e300)))]
#[case::double_underscore(Some(PingSetting::Text("1__0".to_owned())))]
#[case::leading_underscore(Some(PingSetting::Text("_10".to_owned())))]
#[case::trailing_underscore(Some(PingSetting::Text("10_".to_owned())))]
fn coerce_rejects_unusable_settings(#[case] setting: Option<PingSetting>) {
    assert_eq!(coerce_keepalive_interval(setting.as_ref()), None);
}

#[rstest]
#[case::number(Some(PingSetting::Seconds(5.0)), Some(Duration::from_secs(5)))]
#[case::integer_text(Some(PingSetting::Text("5".to_owned())), Some(Duration::from_secs(5)))]
#[case::padded_decimal(Some(PingSetting::Text(" 2.5 ".to_owned())), Some(Duration::from_millis(2500)))]
#[case::exponent(Some(PingSetting::Text("1e1".to_owned())), Some(Duration::from_secs(10)))]
#[case::underscored(Some(PingSetting::Text("1_0".to_owned())), Some(Duration::from_secs(10)))]
#[case::signed_text(Some(PingSetting::Text("+0.5".to_owned())), Some(Duration::from_millis(500)))]
#[case::leading_dot(Some(PingSetting::Text(".25".to_owned())), Some(Duration::from_millis(250)))]
fn coerce_returns_the_interval(
    #[case] setting: Option<PingSetting>,
    #[case] expected: Option<Duration>,
) {
    assert_eq!(coerce_keepalive_interval(setting.as_ref()), expected);
}

#[rstest]
#[case::number_and_float_agree(
    r#"[{"litellm_params": {"keepalive_seconds": 5}}, {"litellm_params": {"keepalive_seconds": 5.0}}]"#,
    Some(Duration::from_secs(5))
)]
#[case::all_agree_without_global(
    r#"[{"litellm_params": {"keepalive_seconds": 2}}, {"litellm_params": {"keepalive_seconds": 2}}]"#,
    Some(Duration::from_secs(2))
)]
#[case::text_and_number_differ(
    r#"[{"litellm_params": {"keepalive_seconds": 5}}, {"litellm_params": {"keepalive_seconds": "5"}}]"#,
    Some(Duration::from_secs(7))
)]
#[case::two_numbers_differ(
    r#"[{"litellm_params": {"keepalive_seconds": 5}}, {"litellm_params": {"keepalive_seconds": 6}}]"#,
    Some(Duration::from_secs(7))
)]
#[case::one_deployment_has_no_value(
    r#"[{"litellm_params": {"keepalive_seconds": 5}}, {"litellm_params": {}}]"#,
    Some(Duration::from_secs(7))
)]
#[case::only_deployment_has_no_value(r#"[{"litellm_params": {}}]"#, Some(Duration::from_secs(7)))]
#[case::not_an_object(r#"["deployment"]"#, Some(Duration::from_secs(7)))]
#[case::bool_is_not_a_number(
    r#"[{"litellm_params": {"keepalive_seconds": true}}]"#,
    Some(Duration::from_secs(7))
)]
#[case::empty_list("[]", Some(Duration::from_secs(7)))]
#[case::shared_zero_disables(
    r#"[{"litellm_params": {"keepalive_seconds": 0}}, {"litellm_params": {"keepalive_seconds": 0}}]"#,
    None
)]
#[case::shared_zero_text_disables(r#"[{"litellm_params": {"keepalive_seconds": "0"}}]"#, None)]
fn agreed_uses_the_shared_deployment_value_or_the_global_one(
    #[case] list: &str,
    #[case] expected: Option<Duration>,
) {
    let deployments = parse_deployments(list);
    let global = Some(PingSetting::Seconds(7.0));
    assert_eq!(
        agreed_keepalive_interval(&deployments, global.as_ref()),
        expected
    );
}

#[test]
fn agreed_without_a_global_value_and_disagreeing_deployments_is_none() {
    let deployments = parse_deployments(
        r#"[{"litellm_params": {"keepalive_seconds": 5}}, {"litellm_params": {"keepalive_seconds": 6}}]"#,
    );
    assert_eq!(agreed_keepalive_interval(&deployments, None), None);
}

#[rstest]
#[case::true_bool(r#"{"stream": true, "model": "gpt"}"#, Some(Duration::from_secs(5)))]
#[case::missing(r#"{"model": "gpt"}"#, None)]
#[case::false_bool(r#"{"stream": false, "model": "gpt"}"#, None)]
#[case::string_true(r#"{"stream": "true", "model": "gpt"}"#, None)]
#[case::number_one(r#"{"stream": 1, "model": "gpt"}"#, None)]
#[case::null(r#"{"stream": null, "model": "gpt"}"#, None)]
fn ttft_interval_only_applies_to_a_true_json_stream_flag(
    #[case] request: &str,
    #[case] expected: Option<Duration>,
) {
    let deployments = parse_deployments(r#"[{"litellm_params": {"keepalive_seconds": 5}}]"#);
    let global = Some(PingSetting::Seconds(7.0));
    assert_eq!(
        ttft_keepalive_interval(&json(request), &deployments, global.as_ref()),
        expected
    );
}

#[test]
fn ttft_interval_falls_back_to_the_global_value_without_deployments() {
    let global = Some(PingSetting::Seconds(7.0));
    assert_eq!(
        ttft_keepalive_interval(&json(r#"{"stream": true}"#), &[], global.as_ref()),
        Some(Duration::from_secs(7))
    );
}

struct FakeLookup {
    deployments: Vec<JsonValue>,
    calls: Mutex<Vec<String>>,
}

impl FakeLookup {
    fn new(deployments: Vec<JsonValue>) -> Self {
        Self {
            deployments,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("lookup lock").clone()
    }
}

impl DeploymentLookup for FakeLookup {
    fn deployments_for(&self, model_name: &str) -> Vec<JsonValue> {
        self.calls
            .lock()
            .expect("lookup lock")
            .push(model_name.to_owned());
        self.deployments.clone()
    }
}

#[test]
fn ttft_interval_for_looks_up_a_string_model() {
    let lookup = FakeLookup::new(parse_deployments(
        r#"[{"litellm_params": {"keepalive_seconds": 5}}]"#,
    ));
    let global = Some(PingSetting::Seconds(7.0));
    let interval = ttft_interval_for(
        Some(&lookup),
        &json(r#"{"stream": true, "model": "gpt-4o"}"#),
        global.as_ref(),
    );
    assert_eq!(interval, Some(Duration::from_secs(5)));
    assert_eq!(lookup.calls(), vec!["gpt-4o".to_owned()]);
}

#[test]
fn ttft_interval_for_without_a_router_uses_the_global_value() {
    let global = Some(PingSetting::Seconds(7.0));
    assert_eq!(
        ttft_interval_for(
            None,
            &json(r#"{"stream": true, "model": "gpt-4o"}"#),
            global.as_ref()
        ),
        Some(Duration::from_secs(7))
    );
}

#[rstest]
#[case::not_a_string_model(r#"{"stream": true, "model": 7}"#)]
#[case::missing_model(r#"{"stream": true}"#)]
fn ttft_interval_for_does_not_look_up_an_unusable_model(#[case] request: &str) {
    let lookup = FakeLookup::new(parse_deployments(
        r#"[{"litellm_params": {"keepalive_seconds": 5}}]"#,
    ));
    let global = Some(PingSetting::Seconds(7.0));
    assert_eq!(
        ttft_interval_for(Some(&lookup), &json(request), global.as_ref()),
        Some(Duration::from_secs(7))
    );
    assert_eq!(lookup.calls(), Vec::<String>::new());
}

#[test]
fn ttft_interval_for_is_none_when_not_streaming() {
    let lookup = FakeLookup::new(parse_deployments(
        r#"[{"litellm_params": {"keepalive_seconds": 5}}]"#,
    ));
    let global = Some(PingSetting::Seconds(7.0));
    assert_eq!(
        ttft_interval_for(
            Some(&lookup),
            &json(r#"{"stream": false, "model": "gpt-4o"}"#),
            global.as_ref()
        ),
        None
    );
    assert_eq!(lookup.calls(), Vec::<String>::new());
}
