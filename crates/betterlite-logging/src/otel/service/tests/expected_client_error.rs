use super::fixtures::{error_config, request_data};
use super::support::{Rig, START, attribute, text_attribute};
use crate::logging_metadata::exception::ExceptionFacts;
use crate::otel::legacy_policy::error_recording::ERROR_STACK_TRACE;
use crate::otel::service::post_call::{
    PostCallFailure, ProxyCallKey, async_post_call_failure_hook,
};

const TRACEBACK: &str = "Traceback (most recent call last):\n  raise AuthenticationError\n";

// a provider-originated error keeps its traceback; a proxy 4xx is an expected client error and drops it
fn facts_for(llm_provider: Option<&str>) -> ExceptionFacts {
    ExceptionFacts {
        class_name: "AuthenticationError".to_owned(),
        message: Some("bad key".to_owned()),
        status_code: Some(401),
        llm_provider: llm_provider.map(str::to_owned),
        traceback_lines: vec![
            "Traceback (most recent call last):\n".to_owned(),
            "  raise AuthenticationError\n".to_owned(),
        ],
        ..ExceptionFacts::default()
    }
}

#[rstest::rstest]
#[case::expected_client_error_omits_traceback(None, None)]
#[case::provider_error_keeps_traceback(Some("openai"), Some(TRACEBACK))]
fn otel_legacy_service_callbacks_behavior_tests_behavior(
    #[case] llm_provider: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let data = request_data("call-e", vec![]);
    let facts = facts_for(llm_provider);
    let key = ProxyCallKey {
        parent_span: Some(&parent),
        team_id: None,
        team_alias: None,
    };
    let config = error_config();
    let call = PostCallFailure {
        request_data: &data,
        exception: Some(&facts),
        key: &key,
        traceback: None,
        info: &config,
    };
    async_post_call_failure_hook(&lifecycle, &call).expect("failure hook");
    let server = rig.span("proxy");
    match expected {
        Some(text) => assert_eq!(text_attribute(&server, ERROR_STACK_TRACE), text),
        None => assert!(attribute(&server, ERROR_STACK_TRACE).is_none()),
    }
}
