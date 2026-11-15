use rstest::rstest;
use serde_json::Value;

use super::support::{built, options};
use crate::spend_event::input::CachePresence;

const KWARGS: &str = r#"{
    "standard_logging_object": {"id": "sl-1", "model_parameters": {"temperature": 0.2},
        "messages": [{"role": "user"}], "response": {"x": 1}, "model": "gpt-4"},
    "litellm_params": {"proxy_server_request": {"body": {"q": 1}}}
}"#;

#[rstest]
#[case::bodies_dropped(false, false)]
#[case::bodies_kept(true, true)]
fn applies_the_body_policy(#[case] store_bodies: bool, #[case] kept: bool) {
    let event = built(KWARGS, &options(store_bodies, CachePresence::On));
    let logging = event["standard_logging_object"].as_object().unwrap();
    assert!(!logging.contains_key("model_parameters"));
    assert_eq!(logging.contains_key("messages"), kept);
    assert_eq!(logging.contains_key("response"), kept);
    assert_eq!(logging["id"], "sl-1");
    assert_eq!(logging["model"], "gpt-4");
    let request = &event["litellm_params"]["proxy_server_request"];
    assert_eq!(!request.is_null(), kept);
}

#[test]
fn a_missing_standard_logging_object_stays_null() {
    let event = built("{}", &options(true, CachePresence::On));
    assert_eq!(event["standard_logging_object"], Value::Null);
    assert_eq!(event["litellm_params"]["proxy_server_request"], Value::Null);
}
