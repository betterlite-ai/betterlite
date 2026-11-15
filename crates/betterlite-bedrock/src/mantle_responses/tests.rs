#![cfg(test)]

use std::sync::Arc;

use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;
use secrecy::ExposeSecret;

use crate::mantle_responses::auth::{EnvironmentParams, validate_environment};
use crate::mantle_responses::config::{
    CUSTOM_LLM_PROVIDER, DEFAULT_USE_OPENAI_PATH, SUPPORTS_NATIVE_FILE_SEARCH,
    SUPPORTS_NATIVE_WEBSOCKET,
};
use crate::mantle_responses::url::{CompleteUrlParams, complete_url};

mod auth_cases;
mod hoisting_cases;
mod model_name_cases;
mod option_cases;
mod params_400_cases;
mod project_header_cases;
mod support;
mod tool_cases;
mod url_cases;
mod warn_drop_cases;

#[derive(Debug, Clone, Copy)]
enum Scenario {
    Url,
    ResponseOptions,
    ModelName,
    UnsupportedParams400,
    ResponseAuth,
    UnsupportedParamsWarnDrop,
    ProjectHeader,
    ToolFilter,
    InputHoisting,
}

#[rstest]
#[case::mantle_responses_url(Scenario::Url)]
#[case::mantle_response_options(Scenario::ResponseOptions)]
#[case::mantle_model_name(Scenario::ModelName)]
#[case::unsupported_params_400(Scenario::UnsupportedParams400)]
#[case::mantle_response_auth(Scenario::ResponseAuth)]
#[case::unsupported_params_warn_drop(Scenario::UnsupportedParamsWarnDrop)]
#[case::openai_project_header(Scenario::ProjectHeader)]
#[case::mantle_tool_filter(Scenario::ToolFilter)]
#[case::mantle_input_hoisting(Scenario::InputHoisting)]
fn mantle_responses_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::Url => url_cases::mantle_responses_url(),
        Scenario::ResponseOptions => option_cases::mantle_response_options(),
        Scenario::ModelName => model_name_cases::mantle_model_name(),
        Scenario::UnsupportedParams400 => params_400_cases::unsupported_params_400(),
        Scenario::ResponseAuth => auth_cases::mantle_response_auth(),
        Scenario::UnsupportedParamsWarnDrop => warn_drop_cases::unsupported_params_warn_drop(),
        Scenario::ProjectHeader => project_header_cases::openai_project_header(),
        Scenario::ToolFilter => tool_cases::mantle_tool_filter(),
        Scenario::InputHoisting => hoisting_cases::mantle_input_hoisting(),
    }
}

#[test]
fn mantle_responses_credential_identity_isolation() {
    let mut first_globals = support::globals();
    first_globals.mantle_api_key = Some(support::secret("first-fixture-key"));
    let mut second_globals = support::globals();
    second_globals.aws_bearer_token = Some(support::secret("second-fixture-key"));
    let first = support::authorization(EnvironmentParams::default(), &first_globals);
    let second = support::authorization(EnvironmentParams::default(), &second_globals);
    assert_eq!(first.as_deref(), Some("Bearer first-fixture-key"));
    assert_eq!(second.as_deref(), Some("Bearer second-fixture-key"));
    let first_cache = support::credential_cache();
    let second_cache = support::credential_cache();
    assert!(!Arc::ptr_eq(&first_cache, &second_cache));
    let first_config = support::config_with_cache(Arc::clone(&first_cache));
    let second_config = support::config_with_cache(Arc::clone(&second_cache));
    assert!(!format!("{first_config:?}").contains("second-fixture-key"));
    assert!(!format!("{second_config:?}").contains("first-fixture-key"));
    let shared = support::secret("shared-fixture-key");
    let params = EnvironmentParams {
        api_key: Some(&shared),
        project_id: None,
    };
    assert_eq!(
        support::authorization(params, &second_globals).as_deref(),
        Some("Bearer shared-fixture-key")
    );
    let mut untouched = support::headers();
    validate_environment(&mut untouched, EnvironmentParams::default(), &first_globals);
    assert!(!untouched.contains_key("OpenAI-Project"));
}

#[test]
fn mantle_responses_secret_safe_failure() {
    let secrets = ["fixture-mantle-key", "fixture-standard-token"];
    let mut globals = support::globals();
    globals.mantle_api_key = Some(support::secret(secrets[0]));
    globals.aws_bearer_token = Some(support::secret(secrets[1]));
    let api_key = support::secret(secrets[0]);
    let params = EnvironmentParams {
        api_key: Some(&api_key),
        project_id: None,
    };
    assert_eq!(
        support::authorization(params, &globals).as_deref(),
        Some("Bearer fixture-mantle-key")
    );
    let failure = support::map("openai.gpt-oss-120b", r#"{"service_tier": "flex"}"#, false)
        .expect_err("a flex tier is rejected");
    let region_params = CompleteUrlParams {
        api_base: None,
        aws_region_name: Some("US-EAST-1"),
    };
    let region = complete_url(&support::config(true), region_params, &globals)
        .expect_err("an unknown region is rejected");
    let config = support::config(true);
    let rendered = format!("{params:?} {globals:?} {config:?} {failure} {failure:?} {region}");
    assert!(rendered.contains("MantleResponsesConfig { use_openai_path: true, .. }"));
    assert!(!rendered.contains(secrets[0]));
    assert!(!rendered.contains(secrets[1]));
    assert_eq!(CUSTOM_LLM_PROVIDER, ProviderId::BedrockMantle);
    assert_eq!(
        [
            DEFAULT_USE_OPENAI_PATH,
            SUPPORTS_NATIVE_FILE_SEARCH,
            SUPPORTS_NATIVE_WEBSOCKET
        ],
        [true, false, false]
    );
    let exposed = globals
        .mantle_api_key
        .as_ref()
        .map(ExposeSecret::expose_secret);
    assert_eq!(exposed, Some(secrets[0]));
}
