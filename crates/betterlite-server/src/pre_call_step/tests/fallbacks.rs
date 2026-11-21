use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::support::{Fake, rate_limited};
use super::{
    Behavior, Call, PreCallError, PreCallOutput, TestKey, behavior, boom, data_from, hook_count,
    input_for, parts, step_for, step_without_router, text_at,
};
use crate::pre_call::route_type::RouteType;
use crate::pre_call_step::PreCall as _;

fn fallback_script(models: &[&str], limited: &[&str]) -> Behavior {
    Behavior {
        fallbacks: Some(
            models
                .iter()
                .map(|model| JsonValue::String((*model).to_owned()))
                .collect(),
        ),
        fallback_models: Some(models.iter().map(|model| (*model).to_owned()).collect()),
        rate_limited_models: limited.iter().map(|model| (*model).to_owned()).collect(),
        ..behavior()
    }
}

async fn fallback_run(
    script: Behavior,
    fixture: &str,
    key: TestKey,
) -> (Vec<Call>, Result<PreCallOutput, PreCallError>) {
    let fake = Arc::new(Fake::new(script));
    let step = step_for(&fake);
    let request = parts("/chat/completions", None);
    let input = input_for(fixture, &request, &key, RouteType::Completion);
    let result = step.pre_call(input).await;
    let events = fake.events();
    (events, result)
}

#[tokio::test]
async fn fallback_returns_the_first_failure_without_fallbacks() {
    let script = Behavior {
        rate_limited_models: vec![String::from("gpt")],
        ..behavior()
    };
    let (events, result) = fallback_run(script, r#"{"model": "gpt"}"#, TestKey::default()).await;
    let Err(error) = result else {
        panic!("the rate limit must surface")
    };
    assert_eq!(error.failure, rate_limited());
    assert!(events.contains(&Call::Fallbacks));
    assert_eq!(hook_count(&events), 1);
    assert!(
        !events
            .iter()
            .any(|call| matches!(call, Call::FallbackModels(_, _)))
    );
}

#[tokio::test]
async fn fallback_returns_the_failure_when_disabled() {
    let script = fallback_script(&["a"], &["gpt"]);
    let fixture = r#"{"model": "gpt", "disable_fallbacks": true}"#;
    let (events, result) = fallback_run(script, fixture, TestKey::default()).await;
    let Err(error) = result else {
        panic!("the rate limit must surface")
    };
    assert_eq!(error.failure, rate_limited());
    assert!(
        !events
            .iter()
            .any(|call| matches!(call, Call::FallbackModels(_, _)))
    );
}

#[tokio::test]
async fn fallback_uses_the_key_router_settings() {
    let mut script = fallback_script(&["key-model"], &["gpt"]);
    script.fallbacks = Some(vec![JsonValue::String(String::from("router-model"))]);
    let key = TestKey {
        aliases: None,
        router_settings: Some(JsonValue::Object(data_from(
            r#"{"fallbacks": ["key-model"]}"#,
        ))),
        team: None,
    };
    let (events, result) = fallback_run(script, r#"{"model": "gpt"}"#, key).await;
    result.unwrap();
    assert!(events.contains(&Call::FallbackModels(
        String::from("gpt"),
        vec![String::from("key-model")],
    )));
    assert!(!events.contains(&Call::Fallbacks));
}

#[tokio::test]
async fn fallback_moves_on_after_a_second_rate_limit() {
    let script = fallback_script(&["a", "b"], &["gpt", "a"]);
    let (events, result) = fallback_run(script, r#"{"model": "gpt"}"#, TestKey::default()).await;
    let output = result.unwrap();
    assert_eq!(text_at(&output.data, "model"), "b");
    assert_eq!(hook_count(&events), 3);
}

#[tokio::test]
async fn fallback_returns_a_successful_fallback() {
    let script = fallback_script(&["a"], &["gpt"]);
    let (events, result) = fallback_run(script, r#"{"model": "gpt"}"#, TestKey::default()).await;
    let output = result.unwrap();
    assert_eq!(text_at(&output.data, "model"), "a");
    assert_eq!(hook_count(&events), 2);
}

#[tokio::test]
async fn fallback_returns_the_original_error_after_the_loop() {
    let script = fallback_script(&["a"], &["gpt", "a"]);
    let (_, result) = fallback_run(script, r#"{"model": "gpt"}"#, TestKey::default()).await;
    let Err(error) = result else {
        panic!("every fallback is rate limited")
    };
    assert_eq!(error.failure, rate_limited());
    assert_eq!(text_at(&error.data, "model"), "gpt");
}

#[tokio::test]
async fn fallback_keeps_the_first_tag_set() {
    let mut script = fallback_script(&["a"], &["gpt"]);
    script.skip_budget = false;
    script.tag_on_prepare = Some(String::from("prepared"));
    let (events, result) = fallback_run(script, r#"{"model": "gpt"}"#, TestKey::default()).await;
    let output = result.unwrap();
    assert_eq!(text_at(&output.data, "model"), "a");
    assert!(events.contains(&Call::Tags(vec![String::from("prepared-a")])));
    assert!(!events.contains(&Call::Tags(vec![String::from("prepared-gpt")])));
}

#[rstest]
#[case("\"\"")]
#[case("0")]
#[case("[]")]
#[case("{}")]
#[tokio::test]
async fn fallback_ignores_a_falsey_disable_flag(#[case] flag: &str) {
    let script = fallback_script(&["a"], &["gpt"]);
    let fixture = format!(r#"{{"model": "gpt", "disable_fallbacks": {flag}}}"#);
    let (_, result) = fallback_run(script, &fixture, TestKey::default()).await;
    result.unwrap();
}

#[tokio::test]
async fn fallback_returns_another_failure_with_the_rate_limited_data() {
    let mut script = fallback_script(&["a"], &["gpt"]);
    script.fail_models = vec![String::from("a")];
    let (events, result) = fallback_run(script, r#"{"model": "gpt"}"#, TestKey::default()).await;
    let Err(error) = result else {
        panic!("the fallback failure must surface")
    };
    assert_eq!(error.failure, boom());
    assert_eq!(text_at(&error.data, "model"), "gpt");
    assert_eq!(hook_count(&events), 2);
}

#[tokio::test]
async fn fallback_returns_a_non_rate_limit_failure_unchanged() {
    let mut script = fallback_script(&["a"], &[]);
    script.fail_models = vec![String::from("gpt")];
    let (events, result) = fallback_run(script, r#"{"model": "gpt"}"#, TestKey::default()).await;
    let Err(error) = result else {
        panic!("a non-rate-limit failure must surface")
    };
    assert_eq!(error.failure, boom());
    assert_eq!(text_at(&error.data, "model"), "gpt");
    assert_eq!(hook_count(&events), 1);
    assert!(
        !events
            .iter()
            .any(|call| matches!(call, Call::FallbackModels(_, _)))
    );
}

#[tokio::test]
async fn fallback_returns_the_failure_without_a_router() {
    let script = fallback_script(&["a"], &["gpt"]);
    let fake = Arc::new(Fake::new(script));
    let step = step_without_router(&fake);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let input = input_for(r#"{"model": "gpt"}"#, &request, &key, RouteType::Completion);
    let result = step.pre_call(input).await;
    let Err(error) = result else {
        panic!("the rate limit must surface")
    };
    assert_eq!(error.failure, rate_limited());
    assert!(!fake.events().contains(&Call::Fallbacks));
}

#[tokio::test]
async fn fallback_returns_the_failure_when_models_do_not_resolve() {
    let mut script = fallback_script(&["a"], &["gpt"]);
    script.fallback_models = None;
    let (events, result) = fallback_run(script, r#"{"model": "gpt"}"#, TestKey::default()).await;
    let Err(error) = result else {
        panic!("the rate limit must surface")
    };
    assert_eq!(error.failure, rate_limited());
    assert_eq!(text_at(&error.data, "model"), "gpt");
    assert!(events.contains(&Call::FallbackModels(
        String::from("gpt"),
        vec![String::from("a")],
    )));
}

#[tokio::test]
async fn fallback_returns_the_failure_when_the_model_is_not_a_string() {
    let mut script = fallback_script(&["a"], &[]);
    script.hook_failure = Some(rate_limited());
    let (events, result) = fallback_run(script, r#"{"model": null}"#, TestKey::default()).await;
    let Err(error) = result else {
        panic!("a null model cannot fall back")
    };
    assert_eq!(error.failure, rate_limited());
    assert!(error.data.get("model").is_some_and(JsonValue::is_null));
    assert_eq!(hook_count(&events), 1);
    assert!(
        !events
            .iter()
            .any(|call| matches!(call, Call::FallbackModels(_, _)))
    );
}
