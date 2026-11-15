#![cfg(test)]

use betterlite_config::sections::batch_provider_list::Settings as ConfiguredSettings;
use betterlite_openai::batches::sdk::{Timeout, TimeoutPhases};
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::batch_provider_list::call::{
    AzureBatchListCall, BatchListProviderCall, OpenAiBatchListCall, VertexBatchListCall,
    XaiBatchListCall,
};
use crate::api::batch_provider_list::error::BatchProviderListError;
use crate::api::batch_provider_list::{
    BatchListProvider, BatchListTimeoutValue, BatchProviderList, BatchProviderListInput,
    BatchProviderListResult,
};
use crate::api::resource_sdk_settings::ResourceSdkSettings;
use crate::genai_setup::params::VertexCredentials;

fn service() -> BatchProviderList {
    service_with(ConfiguredSettings::default())
}

fn service_with(settings: ConfiguredSettings) -> BatchProviderList {
    BatchProviderList {
        settings: ResourceSdkSettings::from(settings),
    }
}

fn configured() -> ConfiguredSettings {
    ConfiguredSettings {
        api_base: None,
        api_version: None,
        api_key: None,
        openai_key: None,
        azure_key: None,
        azure_ad_token: None,
        organization: None,
        vertex_project: None,
        vertex_location: None,
        openai_base_url: None,
        openai_api_base: None,
        openai_api_key: None,
        openai_organization: None,
        azure_api_base: None,
        azure_api_version: None,
        azure_openai_api_key: None,
        azure_api_key: None,
        vertexai_project: None,
        vertexai_location: None,
        vertexai_credentials: None,
    }
}

fn input(provider: BatchListProvider) -> BatchProviderListInput {
    BatchProviderListInput {
        provider,
        after: None,
        limit: None,
        alist_batches: false,
        api_key: None,
        api_base: None,
        api_version: None,
        organization: None,
        timeout: None,
        request_timeout: None,
        max_retries: None,
        vertex_project: None,
        vertex_location: None,
        vertex_credentials: None,
        azure_ad_token: None,
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: None,
        azure_username: None,
        azure_password: None,
    }
}

fn prepared(input: &BatchProviderListInput) -> BatchProviderListResult {
    service()
        .run(input)
        .expect("the input prepares a provider call")
}

fn prepared_with(
    settings: ConfiguredSettings,
    input: &BatchProviderListInput,
) -> BatchProviderListResult {
    service_with(settings)
        .run(input)
        .expect("the call prepares")
}

fn openai_call(result: BatchProviderListResult) -> OpenAiBatchListCall {
    let BatchListProviderCall::OpenAiCompatible(call) = result.call.provider else {
        panic!("expected an OpenAI-compatible call");
    };
    *call
}

fn azure_call(result: BatchProviderListResult) -> AzureBatchListCall {
    let BatchListProviderCall::Azure(call) = result.call.provider else {
        panic!("expected an Azure call");
    };
    *call
}

fn vertex_call(result: BatchProviderListResult) -> VertexBatchListCall {
    let BatchListProviderCall::VertexAi(call) = result.call.provider else {
        panic!("expected a Vertex call");
    };
    *call
}

fn xai_call(result: BatchProviderListResult) -> XaiBatchListCall {
    let BatchListProviderCall::Xai(call) = result.call.provider else {
        panic!("expected an xAI call");
    };
    *call
}

fn exposed(value: Option<&SecretString>) -> Option<&str> {
    value.map(SecretString::expose_secret)
}

#[rstest]
#[case::openai(BatchListProvider::OpenAi)]
#[case::hosted_vllm(BatchListProvider::HostedVllm)]
#[case::litellm_proxy(BatchListProvider::LitellmProxy)]
#[case::vertex_ai(BatchListProvider::VertexAi)]
#[case::xai(BatchListProvider::Xai)]
fn batch_list_after_limit(#[case] provider: BatchListProvider) {
    let mut request = input(provider);
    request.after = Some("cursor-after".to_owned());
    request.limit = Some(25);
    let result = prepared(&request);
    assert_eq!(result.call.window.after.as_deref(), Some("cursor-after"));
    assert_eq!(result.call.window.limit, Some(25));
}

#[test]
fn an_absent_after_and_limit_are_forwarded_as_absent() {
    let result = prepared(&input(BatchListProvider::OpenAi));
    assert_eq!(result.call.window.after, None);
    assert_eq!(result.call.window.limit, None);
}

#[test]
fn openai_call_resolves_the_legacy_parameter_precedence() {
    let mut settings = configured();
    settings.api_key = Some(SecretString::from("global-key"));
    settings.openai_key = Some(SecretString::from("openai-key"));
    settings.openai_api_key = Some(SecretString::from("env-key"));
    settings.api_base = Some("https://global.example".to_owned());
    settings.openai_base_url = Some("https://base-url.example".to_owned());
    settings.organization = Some("global-org".to_owned());
    settings.openai_organization = Some("env-org".to_owned());
    let service = service_with(settings);
    let mut request = input(BatchListProvider::OpenAi);
    request.api_key = Some(SecretString::from("request-key"));
    request.api_base = Some("https://request.example".to_owned());
    request.organization = Some("request-org".to_owned());
    request.max_retries = Some(5);
    let call = openai_call(service.run(&request).expect("the call prepares"));
    assert_eq!(exposed(call.api_key.as_ref()), Some("request-key"));
    assert_eq!(call.api_base, "https://request.example");
    assert_eq!(call.organization.as_deref(), Some("request-org"));
    assert_eq!(call.max_retries, Some(5));
}

#[test]
fn openai_call_falls_back_through_the_global_and_environment_settings() {
    let mut settings = configured();
    settings.api_key = Some(SecretString::from("global-key"));
    settings.openai_key = Some(SecretString::from("openai-key"));
    settings.api_base = Some("https://global.example".to_owned());
    settings.openai_base_url = Some("https://base-url.example".to_owned());
    settings.openai_organization = Some("env-org".to_owned());
    let service = service_with(settings);
    let call = openai_call(
        service
            .run(&input(BatchListProvider::OpenAi))
            .expect("prepares"),
    );
    assert_eq!(exposed(call.api_key.as_ref()), Some("global-key"));
    assert_eq!(call.api_base, "https://global.example");
    assert_eq!(call.organization.as_deref(), Some("env-org"));

    let vllm_call = openai_call(
        service
            .run(&input(BatchListProvider::HostedVllm))
            .expect("prepares"),
    );
    assert_eq!(vllm_call.api_base, "https://global.example");
}

#[test]
fn openai_call_uses_the_default_base_without_any_setting() {
    let call = openai_call(prepared(&input(BatchListProvider::OpenAi)));
    assert_eq!(call.api_base, "https://api.openai.com/v1");
    assert_eq!(exposed(call.api_key.as_ref()), None);
    assert_eq!(call.organization, None);
    assert_eq!(call.max_retries, None);
}

#[test]
fn openai_call_uses_openai_key_alone() {
    let mut settings = configured();
    settings.openai_key = Some(SecretString::from("openai-key"));
    let call = openai_call(prepared_with(settings, &input(BatchListProvider::OpenAi)));
    assert_eq!(exposed(call.api_key.as_ref()), Some("openai-key"));
}

#[test]
fn openai_call_uses_openai_api_key_alone() {
    let mut settings = configured();
    settings.openai_api_key = Some(SecretString::from("env-key"));
    let call = openai_call(prepared_with(settings, &input(BatchListProvider::OpenAi)));
    assert_eq!(exposed(call.api_key.as_ref()), Some("env-key"));
}

#[test]
fn openai_call_uses_openai_base_url_alone() {
    let mut settings = configured();
    settings.openai_base_url = Some("https://base-url.example".to_owned());
    let call = openai_call(prepared_with(settings, &input(BatchListProvider::OpenAi)));
    assert_eq!(call.api_base, "https://base-url.example");
}

#[test]
fn openai_call_uses_openai_api_base_alone() {
    let mut settings = configured();
    settings.openai_api_base = Some("https://api-base.example".to_owned());
    let call = openai_call(prepared_with(settings, &input(BatchListProvider::OpenAi)));
    assert_eq!(call.api_base, "https://api-base.example");
}

#[test]
fn openai_call_uses_organization_alone() {
    let mut settings = configured();
    settings.organization = Some("configured-org".to_owned());
    let call = openai_call(prepared_with(settings, &input(BatchListProvider::OpenAi)));
    assert_eq!(call.organization.as_deref(), Some("configured-org"));
}

#[test]
fn openai_call_prefers_openai_key_over_openai_api_key() {
    let mut settings = configured();
    settings.openai_key = Some(SecretString::from("openai-key"));
    settings.openai_api_key = Some(SecretString::from("env-key"));
    let call = openai_call(prepared_with(settings, &input(BatchListProvider::OpenAi)));
    assert_eq!(exposed(call.api_key.as_ref()), Some("openai-key"));
}

#[test]
fn openai_call_prefers_openai_base_url_over_openai_api_base() {
    let mut settings = configured();
    settings.openai_base_url = Some("https://base-url.example".to_owned());
    settings.openai_api_base = Some("https://api-base.example".to_owned());
    let call = openai_call(prepared_with(settings, &input(BatchListProvider::OpenAi)));
    assert_eq!(call.api_base, "https://base-url.example");
}

#[test]
fn openai_call_prefers_the_configured_organization_over_the_environment() {
    let mut settings = configured();
    settings.organization = Some("configured-org".to_owned());
    settings.openai_organization = Some("env-org".to_owned());
    let call = openai_call(prepared_with(settings, &input(BatchListProvider::OpenAi)));
    assert_eq!(call.organization.as_deref(), Some("configured-org"));
}

#[test]
fn azure_call_resolves_the_azure_precedence() {
    let mut settings = configured();
    settings.api_key = Some(SecretString::from("global-key"));
    settings.azure_key = Some(SecretString::from("azure-key"));
    settings.azure_api_base = Some("https://env-azure.example".to_owned());
    settings.azure_api_version = Some("env-version".to_owned());
    let service = service_with(settings);
    let mut request = input(BatchListProvider::Azure);
    request.api_version = Some("request-version".to_owned());
    request.max_retries = Some(4);
    let call = azure_call(service.run(&request).expect("prepares"));
    assert_eq!(exposed(call.api_key.as_ref()), Some("global-key"));
    assert_eq!(call.api_base.as_deref(), Some("https://env-azure.example"));
    assert_eq!(call.api_version.as_deref(), Some("request-version"));
    assert_eq!(call.max_retries, Some(4));
}

#[test]
fn azure_call_uses_api_version_alone() {
    let mut settings = configured();
    settings.api_version = Some("configured-version".to_owned());
    let call = azure_call(prepared_with(settings, &input(BatchListProvider::Azure)));
    assert_eq!(call.api_version.as_deref(), Some("configured-version"));
}

#[test]
fn azure_call_uses_azure_api_version_alone() {
    let mut settings = configured();
    settings.azure_api_version = Some("env-version".to_owned());
    let call = azure_call(prepared_with(settings, &input(BatchListProvider::Azure)));
    assert_eq!(call.api_version.as_deref(), Some("env-version"));
}

#[test]
fn azure_call_uses_azure_key_alone() {
    let mut settings = configured();
    settings.azure_key = Some(SecretString::from("azure-key"));
    let call = azure_call(prepared_with(settings, &input(BatchListProvider::Azure)));
    assert_eq!(exposed(call.api_key.as_ref()), Some("azure-key"));
}

#[test]
fn azure_call_uses_azure_openai_api_key_alone() {
    let mut settings = configured();
    settings.azure_openai_api_key = Some(SecretString::from("env-openai-key"));
    let call = azure_call(prepared_with(settings, &input(BatchListProvider::Azure)));
    assert_eq!(exposed(call.api_key.as_ref()), Some("env-openai-key"));
}

#[test]
fn azure_call_uses_azure_api_key_alone() {
    let mut settings = configured();
    settings.azure_api_key = Some(SecretString::from("env-azure-key"));
    let call = azure_call(prepared_with(settings, &input(BatchListProvider::Azure)));
    assert_eq!(exposed(call.api_key.as_ref()), Some("env-azure-key"));
}

#[test]
fn azure_call_prefers_api_version_over_azure_api_version() {
    let mut settings = configured();
    settings.api_version = Some("configured-version".to_owned());
    settings.azure_api_version = Some("env-version".to_owned());
    let call = azure_call(prepared_with(settings, &input(BatchListProvider::Azure)));
    assert_eq!(call.api_version.as_deref(), Some("configured-version"));
}

#[test]
fn azure_call_keeps_the_configured_key_order() {
    let mut all_configured = configured();
    all_configured.azure_key = Some(SecretString::from("azure-key"));
    all_configured.azure_openai_api_key = Some(SecretString::from("env-openai-key"));
    all_configured.azure_api_key = Some(SecretString::from("env-azure-key"));
    let call = azure_call(prepared_with(
        all_configured,
        &input(BatchListProvider::Azure),
    ));
    assert_eq!(exposed(call.api_key.as_ref()), Some("azure-key"));

    let mut env_settings = configured();
    env_settings.azure_openai_api_key = Some(SecretString::from("env-openai-key"));
    env_settings.azure_api_key = Some(SecretString::from("env-azure-key"));
    let env_call = azure_call(prepared_with(
        env_settings,
        &input(BatchListProvider::Azure),
    ));
    assert_eq!(exposed(env_call.api_key.as_ref()), Some("env-openai-key"));
}

#[test]
fn azure_call_keeps_the_request_credentials_in_the_litellm_params() {
    let mut request = input(BatchListProvider::Azure);
    request.api_key = Some(SecretString::from("request-key"));
    request.api_base = Some("https://request-azure.example".to_owned());
    request.api_version = Some("request-version".to_owned());
    request.max_retries = Some(2);
    request.azure_ad_token = Some(SecretString::from("ad-token"));
    request.tenant_id = Some("tenant".to_owned());
    request.client_id = Some("client".to_owned());
    request.client_secret = Some(SecretString::from("client-secret"));
    request.azure_scope = Some("scope".to_owned());
    request.azure_username = Some("user".to_owned());
    request.azure_password = Some(SecretString::from("password"));
    let call = azure_call(prepared(&request));
    let params = call.litellm_params;
    assert_eq!(exposed(params.api_key.as_ref()), Some("request-key"));
    assert_eq!(
        params.api_base.as_deref(),
        Some("https://request-azure.example")
    );
    assert_eq!(params.api_version.as_deref(), Some("request-version"));
    assert_eq!(params.max_retries, Some(2));
    assert!(params.timeout.is_none());
    assert_eq!(exposed(params.azure_ad_token.as_ref()), Some("ad-token"));
    assert_eq!(params.tenant_id.as_deref(), Some("tenant"));
    assert_eq!(params.client_id.as_deref(), Some("client"));
    assert_eq!(
        exposed(params.client_secret.as_ref()),
        Some("client-secret")
    );
    assert_eq!(params.azure_scope.as_deref(), Some("scope"));
    assert_eq!(params.azure_username.as_deref(), Some("user"));
    assert_eq!(exposed(params.azure_password.as_ref()), Some("password"));
}

#[test]
fn vertex_call_resolves_the_vertex_precedence() {
    let mut settings = configured();
    settings.vertex_project = Some("global-project".to_owned());
    settings.vertexai_project = Some("env-project".to_owned());
    settings.vertexai_location = Some("env-location".to_owned());
    settings.vertexai_credentials = Some(SecretString::from("env-credentials"));
    let service = service_with(settings);
    let mut request = input(BatchListProvider::VertexAi);
    request.api_base = Some("https://vertex.example".to_owned());
    request.max_retries = Some(3);
    let call = vertex_call(service.run(&request).expect("prepares"));
    assert_eq!(call.api_base, "https://vertex.example");
    assert_eq!(call.vertex_project.as_deref(), Some("global-project"));
    assert_eq!(call.vertex_location.as_deref(), Some("env-location"));
    assert_eq!(
        call.vertex_credentials,
        Some(VertexCredentials::Path("env-credentials".to_owned()))
    );
    assert_eq!(call.max_retries, Some(3));
}

#[test]
fn vertex_call_uses_vertex_location_alone() {
    let mut settings = configured();
    settings.vertex_location = Some("configured-location".to_owned());
    let call = vertex_call(prepared_with(settings, &input(BatchListProvider::VertexAi)));
    assert_eq!(call.vertex_location.as_deref(), Some("configured-location"));
}

#[test]
fn vertex_call_prefers_vertex_location_over_vertexai_location() {
    let mut settings = configured();
    settings.vertex_location = Some("configured-location".to_owned());
    settings.vertexai_location = Some("env-location".to_owned());
    let call = vertex_call(prepared_with(settings, &input(BatchListProvider::VertexAi)));
    assert_eq!(call.vertex_location.as_deref(), Some("configured-location"));
}

#[test]
fn vertex_call_uses_vertexai_project_alone() {
    let mut settings = configured();
    settings.vertexai_project = Some("env-project".to_owned());
    let call = vertex_call(prepared_with(settings, &input(BatchListProvider::VertexAi)));
    assert_eq!(call.vertex_project.as_deref(), Some("env-project"));
}

#[test]
fn vertex_call_prefers_a_request_credentials_path_over_the_environment() {
    let mut settings = configured();
    settings.vertexai_credentials = Some(SecretString::from("env-credentials"));
    let service = service_with(settings);
    let mut request = input(BatchListProvider::VertexAi);
    request.vertex_credentials = Some(Json::Text("request-credentials.json".to_owned()));
    let call = vertex_call(service.run(&request).expect("prepares"));
    assert_eq!(
        call.vertex_credentials,
        Some(VertexCredentials::Path(
            "request-credentials.json".to_owned()
        ))
    );
}

#[test]
fn vertex_call_carries_a_service_account_credentials_object() {
    let mut settings = configured();
    settings.vertexai_credentials = Some(SecretString::from("env-credentials"));
    let service = service_with(settings);
    let source = r#"{
        "provider": "vertex_ai",
        "vertex_credentials": {"type": "service_account", "project_id": "example"}
    }"#;
    let request: BatchProviderListInput =
        serde_json::from_str(source).expect("the object form parses");
    let call = vertex_call(service.run(&request).expect("the service prepares"));
    let Some(VertexCredentials::ServiceAccount(account)) = call.vertex_credentials else {
        panic!("expected a service-account credentials object");
    };
    assert_eq!(
        account.get("type").and_then(Json::as_str),
        Some("service_account")
    );
    assert_eq!(
        account.get("project_id").and_then(Json::as_str),
        Some("example")
    );
}

#[test]
fn an_explicit_null_credentials_value_is_absent() {
    let source = r#"{"provider": "vertex_ai", "vertex_credentials": null}"#;
    let request: BatchProviderListInput = serde_json::from_str(source).expect("input parses");
    assert!(vertex_call(prepared(&request)).vertex_credentials.is_none());
}

#[rstest]
#[case::number("7")]
#[case::boolean("true")]
#[case::list("[]")]
fn vertex_call_rejects_credentials_that_are_neither_text_nor_object(#[case] value: &str) {
    let source = format!(r#"{{"provider": "vertex_ai", "vertex_credentials": {value}}}"#);
    let request: BatchProviderListInput = serde_json::from_str(&source).expect("input parses");
    let error = service()
        .run(&request)
        .expect_err("the credentials are rejected");
    assert!(matches!(
        error,
        BatchProviderListError::InvalidVertexCredentials
    ));
}

#[test]
fn vertex_call_uses_an_empty_base_without_one_configured() {
    let call = vertex_call(prepared(&input(BatchListProvider::VertexAi)));
    assert_eq!(call.api_base, "");
    assert_eq!(call.vertex_project, None);
    assert_eq!(call.vertex_location, None);
    assert!(call.vertex_credentials.is_none());
}

#[test]
fn xai_call_uses_only_the_request_parameters() {
    let mut settings = configured();
    settings.api_key = Some(SecretString::from("global-key"));
    settings.api_base = Some("https://global.example".to_owned());
    let service = service_with(settings);
    let call = xai_call(
        service
            .run(&input(BatchListProvider::Xai))
            .expect("prepares"),
    );
    assert_eq!(call.api_base, None);
    assert_eq!(exposed(call.api_key.as_ref()), None);
}

#[rstest]
#[case::absent(None, None, 600.0_f64)]
#[case::request_timeout(None, Some(30.0_f64), 30.0_f64)]
#[case::explicit_timeout(Some(BatchListTimeoutValue::Seconds(12.5)), None, 12.5_f64)]
#[case::timeout_wins(Some(BatchListTimeoutValue::Seconds(12.5)), Some(30.0_f64), 12.5_f64)]
#[case::explicit_zero(Some(BatchListTimeoutValue::Seconds(0.0)), None, 0.0_f64)]
#[case::text(Some(BatchListTimeoutValue::Text("7.5".to_owned())), None, 7.5_f64)]
#[case::text_with_spaces(Some(BatchListTimeoutValue::Text(" 7.5 ".to_owned())), None, 7.5_f64)]
fn timeout_resolution(
    #[case] timeout: Option<BatchListTimeoutValue>,
    #[case] request_timeout: Option<f64>,
    #[case] expected: f64,
) {
    let mut request = input(BatchListProvider::OpenAi);
    request.timeout = timeout;
    request.request_timeout = request_timeout;
    let result = prepared(&request);
    assert_eq!(result.call.window.timeout, Timeout::Seconds(expected));
}

#[test]
fn a_structured_timeout_passes_through_for_openai_and_azure() {
    let phases = TimeoutPhases {
        connect: Some(1.0_f64),
        read: Some(2.0_f64),
        write: Some(3.0_f64),
        pool: Some(4.0_f64),
    };
    let mut request = input(BatchListProvider::OpenAi);
    request.timeout = Some(BatchListTimeoutValue::Phases(phases.clone()));
    assert_eq!(
        prepared(&request).call.window.timeout,
        Timeout::Phases(phases.clone())
    );
    let mut azure_request = input(BatchListProvider::Azure);
    azure_request.timeout = Some(BatchListTimeoutValue::Phases(phases.clone()));
    assert_eq!(
        prepared(&azure_request).call.window.timeout,
        Timeout::Phases(phases)
    );
}

#[rstest]
#[case::vertex(BatchListProvider::VertexAi)]
#[case::xai(BatchListProvider::Xai)]
#[case::hosted_vllm(BatchListProvider::HostedVllm)]
#[case::litellm_proxy(BatchListProvider::LitellmProxy)]
fn a_structured_timeout_uses_its_read_value_elsewhere(#[case] provider: BatchListProvider) {
    let mut request = input(provider);
    request.timeout = Some(BatchListTimeoutValue::Phases(TimeoutPhases {
        connect: Some(1.0_f64),
        read: Some(9.0_f64),
        write: Some(3.0_f64),
        pool: Some(4.0_f64),
    }));
    let result = prepared(&request);
    assert_eq!(result.call.window.timeout, Timeout::Seconds(9.0));
}

#[test]
fn a_structured_timeout_without_a_read_value_uses_the_default() {
    let mut request = input(BatchListProvider::VertexAi);
    request.timeout = Some(BatchListTimeoutValue::Phases(TimeoutPhases {
        connect: Some(1.0_f64),
        read: None,
        write: Some(3.0_f64),
        pool: Some(4.0_f64),
    }));
    assert_eq!(
        prepared(&request).call.window.timeout,
        Timeout::Seconds(600.0)
    );
}

#[rstest]
#[case::text("not-a-number")]
#[case::digit_separator("1_0")]
#[case::nan("nan")]
#[case::infinity("inf")]
fn an_invalid_timeout_text_is_rejected(#[case] text: &str) {
    let mut request = input(BatchListProvider::OpenAi);
    request.timeout = Some(BatchListTimeoutValue::Text(text.to_owned()));
    let error = service()
        .run(&request)
        .expect_err("the timeout is rejected");
    assert!(matches!(
        error,
        BatchProviderListError::InvalidTimeout { .. } | BatchProviderListError::NonFiniteTimeout
    ));
}

#[test]
fn a_non_finite_timeout_number_is_rejected() {
    let mut request = input(BatchListProvider::OpenAi);
    request.timeout = Some(BatchListTimeoutValue::Seconds(f64::NAN));
    let error = service()
        .run(&request)
        .expect_err("the timeout is rejected");
    assert!(matches!(error, BatchProviderListError::NonFiniteTimeout));
}

#[test]
fn non_finite_phases_are_rejected_for_supporting_providers() {
    let mut request = input(BatchListProvider::OpenAi);
    request.timeout = Some(BatchListTimeoutValue::Phases(TimeoutPhases {
        connect: Some(f64::INFINITY),
        read: Some(2.0_f64),
        write: None,
        pool: None,
    }));
    let error = service()
        .run(&request)
        .expect_err("the timeout is rejected");
    assert!(matches!(error, BatchProviderListError::NonFiniteTimeout));
}

#[test]
fn an_unsupported_provider_is_rejected() {
    let request = input(BatchListProvider::Other("bedrock".to_owned()));
    let error = service()
        .run(&request)
        .expect_err("the provider is unsupported");
    let BatchProviderListError::UnsupportedProvider { provider } = error else {
        panic!("expected an unsupported provider error");
    };
    assert_eq!(provider, "bedrock");
}

#[test]
fn alist_batches_marks_the_prepared_call_async() {
    let mut request = input(BatchListProvider::OpenAi);
    request.alist_batches = true;
    let result = prepared(&request);
    assert!(result.is_async);
    let mut sync_request = input(BatchListProvider::OpenAi);
    sync_request.alist_batches = false;
    assert!(!prepared(&sync_request).is_async);
}

#[test]
fn provider_names_match_the_litellm_spelling() {
    assert_eq!(BatchListProvider::OpenAi.name(), "openai");
    assert_eq!(BatchListProvider::HostedVllm.name(), "hosted_vllm");
    assert_eq!(BatchListProvider::LitellmProxy.name(), "litellm_proxy");
    assert_eq!(BatchListProvider::Azure.name(), "azure");
    assert_eq!(BatchListProvider::VertexAi.name(), "vertex_ai");
    assert_eq!(BatchListProvider::Xai.name(), "xai");
    assert_eq!(
        BatchListProvider::Other("bedrock".to_owned()).name(),
        "bedrock"
    );
}

#[rstest]
#[case::openai("openai", BatchListProvider::OpenAi)]
#[case::hosted_vllm("hosted_vllm", BatchListProvider::HostedVllm)]
#[case::litellm_proxy("litellm_proxy", BatchListProvider::LitellmProxy)]
#[case::azure("azure", BatchListProvider::Azure)]
#[case::vertex_ai("vertex_ai", BatchListProvider::VertexAi)]
#[case::xai("xai", BatchListProvider::Xai)]
#[case::other("bedrock", BatchListProvider::Other("bedrock".to_owned()))]
fn provider_names_deserialize_to_their_variant(
    #[case] name: &str,
    #[case] expected: BatchListProvider,
) {
    let source = format!(r#"{{"provider": "{name}"}}"#);
    let request: BatchProviderListInput = serde_json::from_str(&source).expect("input parses");
    assert_eq!(request.provider, expected);
}

#[test]
fn an_absent_provider_defaults_to_openai() {
    let request: BatchProviderListInput = serde_json::from_str("{}").expect("input parses");
    assert_eq!(request.provider, BatchListProvider::OpenAi);
}
