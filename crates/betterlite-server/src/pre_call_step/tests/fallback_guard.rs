use betterlite_config::sections::settings::JsonValue;

use super::support::rate_limited;
use super::{Call, TestKey, fallback_run, fallback_script, hook_count};

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
