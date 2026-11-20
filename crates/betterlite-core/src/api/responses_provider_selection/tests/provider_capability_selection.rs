use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;

use super::{capable_native, configs, json};
use crate::api::responses_provider_selection::error::{
    ENCRYPTED_TASK_MESSAGE, compatibility_failure,
};
use crate::api::responses_provider_selection::provider_config::{
    CompatibilityFailure, ConfigLookup, NativeConfig, ResponsesApiConfig,
    bridges_to_chat_completions, encrypted_task_support_failure, passes_responses_through,
    resolve_provider_config,
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
    assert!(ResponsesApiConfig::Native(capable_native()).supports_encrypted_tasks());
    assert!(
        !ResponsesApiConfig::Native(NativeConfig {
            encrypted_agent_messages: false,
        })
        .supports_encrypted_tasks()
    );
    assert!(!ResponsesApiConfig::OpenAiLike.supports_encrypted_tasks());
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
#[case::bare_responses(r#"{"supported_endpoints":["/responses"]}"#, false)]
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
    assert_eq!(passes_responses_through(&json(model_info)), expected);
}

#[test]
fn the_registry_lookup_wins_over_the_endpoint_marker() {
    let lookup = configs(&[("openai", capable_native())]);
    let model_info = json(r#"{"supported_endpoints":["/v1/responses"]}"#);
    assert_eq!(
        resolve_provider_config(&lookup, "gpt-4o", "openai", &model_info, None),
        Some(ResponsesApiConfig::Native(capable_native()))
    );
}

#[test]
fn only_the_endpoint_marker_selects_the_compatible_config() {
    let lookup = configs(&[("azure", capable_native())]);
    let supported = json(r#"{"supported_endpoints":["/v1/responses"]}"#);
    assert_eq!(
        resolve_provider_config(&lookup, "gpt-4o", "openai", &supported, None),
        Some(ResponsesApiConfig::OpenAiLike)
    );
    let unsupported = json("{}");
    assert_eq!(
        resolve_provider_config(&lookup, "gpt-4o", "openai", &unsupported, None),
        None
    );
}

#[rstest]
#[case::full_request(
    "gpt-4o",
    "openai",
    "https://base.test/v1",
    Some(ResponsesApiConfig::Native(capable_native()))
)]
#[case::other_model("gpt-4o-mini", "openai", "https://base.test/v1", None)]
#[case::other_provider("gpt-4o", "azure", "https://base.test/v1", None)]
#[case::other_base("gpt-4o", "openai", "https://other.test/v1", None)]
fn the_lookup_receives_the_whole_request(
    #[case] model: &str,
    #[case] provider: &str,
    #[case] api_base: &str,
    #[case] expected: Option<ResponsesApiConfig>,
) {
    let lookup = BaseReadingConfigs;
    let model_info = json("{}");
    assert_eq!(
        resolve_provider_config(&lookup, model, provider, &model_info, Some(api_base)),
        expected
    );
}

#[test]
fn the_encrypted_failure_names_itself_on_the_wire() {
    assert_eq!(
        CompatibilityFailure::EncryptedTaskUnsupported.wire_name(),
        "encrypted_task_unsupported"
    );
}

#[test]
fn the_compatibility_failure_is_an_api_connection_error() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    let class = compatibility_failure(
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
