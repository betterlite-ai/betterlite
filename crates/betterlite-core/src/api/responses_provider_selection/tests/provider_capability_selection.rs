use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;

use super::{capable_native, configs, json};
use crate::api::responses_provider_selection::error::{
    ENCRYPTED_TASK_MESSAGE, responses_compatibility_failure,
};
use crate::api::responses_provider_selection::provider_config::{
    CompatibilityFailure, ConfigLookup, ConfigRequest, NativeConfig, ResponsesApiConfig,
    bridges_to_chat_completions, deployment_passes_through_responses,
    encrypted_task_support_failure, resolve_responses_api_provider_config,
};

struct BaseReadingConfigs;

impl ConfigLookup for BaseReadingConfigs {
    fn responses_api_config(
        &self,
        model: &str,
        provider: &str,
        api_base: Option<&str>,
    ) -> Option<NativeConfig> {
        (model == "gpt-4o" && provider == "openai" && api_base == Some("https://base.test/v1"))
            .then_some(capable_native())
    }
}

#[test]
fn a_missing_config_bridges_every_request() {
    assert!(bridges_to_chat_completions(None, false));
    assert!(bridges_to_chat_completions(None, true));
}

#[test]
fn a_resolved_config_bridges_only_when_asked() {
    let native = ResponsesApiConfig::Native(capable_native());
    let compatible = ResponsesApiConfig::OpenAiLike;
    assert!(!bridges_to_chat_completions(Some(&native), false));
    assert!(bridges_to_chat_completions(Some(&native), true));
    assert!(!bridges_to_chat_completions(Some(&compatible), false));
    assert!(bridges_to_chat_completions(Some(&compatible), true));
}

#[test]
fn only_a_capable_native_config_advertises_encrypted_messages() {
    assert!(ResponsesApiConfig::Native(capable_native()).supports_encrypted_agent_messages());
    assert!(
        !ResponsesApiConfig::Native(NativeConfig {
            encrypted_agent_messages: false,
        })
        .supports_encrypted_agent_messages()
    );
    assert!(!ResponsesApiConfig::OpenAiLike.supports_encrypted_agent_messages());
}

#[test]
fn a_bridged_request_never_supports_encrypted_tasks() {
    let capable = ResponsesApiConfig::Native(capable_native());
    let incapable = ResponsesApiConfig::Native(NativeConfig {
        encrypted_agent_messages: false,
    });
    let unsupported = Some(CompatibilityFailure::EncryptedTaskUnsupported);
    assert_eq!(encrypted_task_support_failure(Some(&capable), false), None);
    assert_eq!(
        encrypted_task_support_failure(Some(&capable), true),
        unsupported
    );
    assert_eq!(
        encrypted_task_support_failure(Some(&incapable), false),
        unsupported
    );
    assert_eq!(
        encrypted_task_support_failure(Some(&ResponsesApiConfig::OpenAiLike), false),
        unsupported
    );
    assert_eq!(encrypted_task_support_failure(None, false), unsupported);
}

#[rstest]
#[case::exact(r#"{"supported_endpoints":["/v1/responses"]}"#, true)]
#[case::among_others(
    r#"{"supported_endpoints":["/v1/chat/completions","/v1/responses"]}"#,
    true
)]
#[case::wrong_endpoint(r#"{"supported_endpoints":["/v1/chat/completions"]}"#, false)]
#[case::empty_list(r#"{"supported_endpoints":[]}"#, false)]
#[case::trailing_slash(r#"{"supported_endpoints":["/v1/responses/"]}"#, false)]
#[case::text_value(r#"{"supported_endpoints":"/v1/responses"}"#, false)]
#[case::null_value(r#"{"supported_endpoints":null}"#, false)]
#[case::absent_key(r#"{"id":"dep-1"}"#, false)]
#[case::root_list(r#"["/v1/responses"]"#, false)]
#[case::root_null("null", false)]
fn supported_endpoints_opt_into_the_compatible_config(
    #[case] model_info: &str,
    #[case] expected: bool,
) {
    assert_eq!(
        deployment_passes_through_responses(&json(model_info)),
        expected
    );
}

#[test]
fn the_registry_lookup_wins_over_the_endpoint_marker() {
    let lookup = configs(&[("openai", capable_native())]);
    let model_info = json(r#"{"supported_endpoints":["/v1/responses"]}"#);
    let request = ConfigRequest {
        model: "gpt-4o",
        provider: "openai",
        model_info: &model_info,
        api_base: None,
    };
    assert_eq!(
        resolve_responses_api_provider_config(&lookup, &request),
        Some(ResponsesApiConfig::Native(capable_native()))
    );
}

#[test]
fn only_the_endpoint_marker_selects_the_compatible_config() {
    let lookup = configs(&[("azure", capable_native())]);
    let supported = json(r#"{"supported_endpoints":["/v1/responses"]}"#);
    let request = ConfigRequest {
        model: "gpt-4o",
        provider: "openai",
        model_info: &supported,
        api_base: None,
    };
    assert_eq!(
        resolve_responses_api_provider_config(&lookup, &request),
        Some(ResponsesApiConfig::OpenAiLike)
    );
    let unsupported = json("{}");
    let request = ConfigRequest {
        model: "gpt-4o",
        provider: "openai",
        model_info: &unsupported,
        api_base: None,
    };
    assert_eq!(
        resolve_responses_api_provider_config(&lookup, &request),
        None
    );
}

#[test]
fn the_lookup_receives_the_whole_request() {
    let lookup = BaseReadingConfigs;
    let model_info = json("{}");
    let matching = ConfigRequest {
        model: "gpt-4o",
        provider: "openai",
        model_info: &model_info,
        api_base: Some("https://base.test/v1"),
    };
    assert_eq!(
        resolve_responses_api_provider_config(&lookup, &matching),
        Some(ResponsesApiConfig::Native(capable_native()))
    );
    let other_model = ConfigRequest {
        model: "gpt-4o-mini",
        provider: "openai",
        model_info: &model_info,
        api_base: Some("https://base.test/v1"),
    };
    assert_eq!(
        resolve_responses_api_provider_config(&lookup, &other_model),
        None
    );
    let other_provider = ConfigRequest {
        model: "gpt-4o",
        provider: "azure",
        model_info: &model_info,
        api_base: Some("https://base.test/v1"),
    };
    assert_eq!(
        resolve_responses_api_provider_config(&lookup, &other_provider),
        None
    );
    let other_base = ConfigRequest {
        model: "gpt-4o",
        provider: "openai",
        model_info: &model_info,
        api_base: Some("https://other.test/v1"),
    };
    assert_eq!(
        resolve_responses_api_provider_config(&lookup, &other_base),
        None
    );
}

#[test]
fn the_compatibility_failure_is_an_api_connection_error() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    let class = responses_compatibility_failure(
        CompatibilityFailure::EncryptedTaskUnsupported,
        "gpt-4o",
        Some("openai"),
        &redactor,
    );
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 500);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.code, "500");
    assert_eq!(metadata.openai_code, None);
    assert_eq!(metadata.model.as_deref(), Some("gpt-4o"));
    assert_eq!(metadata.provider.as_deref(), Some("openai"));
    let ErrorClass::ApiConnection(context) = class else {
        panic!("a ValueError dispatches as an api connection error");
    };
    assert!(context.message.ends_with(ENCRYPTED_TASK_MESSAGE));
}
