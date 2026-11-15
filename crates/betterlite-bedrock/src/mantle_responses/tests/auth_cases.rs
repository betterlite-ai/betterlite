use betterlite_models::identity::provider_id::ProviderId;

use crate::bedrock_settings::{ErrorHeaders, error_response_text};
use crate::mantle_responses::auth::{AUTHORIZATION, EnvironmentParams, validate_environment};
use crate::mantle_responses::config::{
    CUSTOM_LLM_PROVIDER, SUPPORTS_NATIVE_FILE_SEARCH, SUPPORTS_NATIVE_WEBSOCKET,
};
use crate::mantle_responses::error::error_from_reply;

use super::support;

pub(super) fn mantle_response_auth() {
    bearer_cases();
    precedence_cases();
    absent_bearer_cases();
    error_class_cases();
    capability_cases();
}

fn bearer_cases() {
    let explicit = support::secret("explicit-fixture-key");
    let mut globals = support::globals();
    globals.mantle_api_key = Some(support::secret("mantle-fixture-key"));
    globals.aws_bearer_token = Some(support::secret("token-fixture-key"));
    let mut headers = support::headers();
    let params = EnvironmentParams {
        api_key: Some(&explicit),
        project_id: None,
    };
    validate_environment(&mut headers, params, &globals);
    assert_eq!(
        support::header(&headers, AUTHORIZATION).as_deref(),
        Some("Bearer explicit-fixture-key")
    );
    let mut mantle_only = support::headers();
    validate_environment(&mut mantle_only, EnvironmentParams::default(), &globals);
    assert_eq!(
        support::header(&mantle_only, AUTHORIZATION).as_deref(),
        Some("Bearer mantle-fixture-key")
    );
    let mut standard = support::globals();
    standard.aws_bearer_token = Some(support::secret("token-fixture-key"));
    let mut token_only = support::headers();
    validate_environment(&mut token_only, EnvironmentParams::default(), &standard);
    assert_eq!(
        support::header(&token_only, AUTHORIZATION).as_deref(),
        Some("Bearer token-fixture-key")
    );
}

fn precedence_cases() {
    let empty = support::secret("");
    let mut globals = support::globals();
    globals.mantle_api_key = Some(support::secret("mantle-fixture-key"));
    let mut headers = support::headers();
    let params = EnvironmentParams {
        api_key: Some(&empty),
        project_id: None,
    };
    validate_environment(&mut headers, params, &globals);
    assert_eq!(
        support::header(&headers, AUTHORIZATION).as_deref(),
        Some("Bearer mantle-fixture-key")
    );
    let explicit = support::secret("explicit-fixture-key");
    let mut override_headers = support::headers();
    let override_params = EnvironmentParams {
        api_key: Some(&explicit),
        project_id: None,
    };
    validate_environment(&mut override_headers, override_params, &globals);
    assert_eq!(
        support::header(&override_headers, AUTHORIZATION).as_deref(),
        Some("Bearer explicit-fixture-key")
    );
}

fn absent_bearer_cases() {
    let globals = support::globals();
    let mut headers = support::headers();
    validate_environment(&mut headers, EnvironmentParams::default(), &globals);
    assert_eq!(headers.len(), 1);
    let mut empty_globals = support::globals();
    empty_globals.aws_bearer_token = Some(support::secret(""));
    let mut empty_headers = support::headers();
    validate_environment(
        &mut empty_headers,
        EnvironmentParams::default(),
        &empty_globals,
    );
    assert_eq!(
        support::header(&empty_headers, AUTHORIZATION).as_deref(),
        Some("Bearer caller-fixture-key")
    );
    let mut cased = support::headers();
    cased.insert("authorization".to_owned(), "Bearer lower-case".to_owned());
    let explicit = support::secret("explicit-fixture-key");
    let params = EnvironmentParams {
        api_key: Some(&explicit),
        project_id: None,
    };
    validate_environment(&mut cased, params, &globals);
    assert_eq!(
        support::header(&cased, "authorization").as_deref(),
        Some("Bearer lower-case")
    );
    assert_eq!(
        support::header(&cased, AUTHORIZATION).as_deref(),
        Some("Bearer explicit-fixture-key")
    );
}

fn error_class_cases() {
    let headers = vec![
        ("x-request-id".to_owned(), "fixture-request".to_owned()),
        ("retry-after".to_owned(), "7".to_owned()),
    ];
    let error =
        error_from_reply(429, "fixture quota exhausted", &headers).expect("fixture headers");
    assert_eq!(error.error.status_code, 429);
    assert_eq!(error.error.message, "fixture quota exhausted");
    assert_eq!(error.error.headers.as_deref(), Some(headers.as_slice()));
    assert_eq!(error.headers, Some(ErrorHeaders::Http(headers.clone())));
    assert_eq!(error.response_headers, headers);
    assert_eq!(error_response_text(&error.response_body), "");
    assert!(!format!("{error:?}").contains("fixture-request"));
    assert!(!error.error.status_code_is_synthesized);
}

fn capability_cases() {
    assert_eq!(CUSTOM_LLM_PROVIDER, ProviderId::BedrockMantle);
    assert_eq!(
        [SUPPORTS_NATIVE_FILE_SEARCH, SUPPORTS_NATIVE_WEBSOCKET],
        [false, false]
    );
}
