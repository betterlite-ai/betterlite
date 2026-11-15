use std::collections::BTreeMap;

use betterlite_config::sections::batch_provider_read::Settings;
use betterlite_openai::batches::records::BatchStatus;
use betterlite_openai::batches::sdk::{Timeout, TimeoutPhases};
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::{
    ProviderStatus, Recorded, RecordedAws, RecordedParams, answer, answered, base_settings,
    configured, failed, input, recorded_calls, request, scripted,
};
use crate::api::api_boundaries::{ApiLimitError, ApiLimits};
use crate::api::batch_provider_read::call::BatchProviderReadCall;
use crate::api::batch_provider_read::error::{BatchProviderReadError, ReadErrorFields};
use crate::api::batch_provider_read::{
    BatchProviderRead, BatchProviderReadInput, BatchReadOptionalParams, BatchReadProvider,
    BatchReadTimeoutValue,
};
use crate::api::resource_sdk_settings;
use crate::genai_setup::params::VertexCredentials;

fn json_value(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON value")
}

fn phases(connect: Option<f64>, read: Option<f64>) -> TimeoutPhases {
    TimeoutPhases {
        connect,
        read,
        write: None,
        pool: None,
    }
}

fn with_params(
    call: BatchProviderReadInput,
    params: BatchReadOptionalParams,
) -> BatchProviderReadInput {
    BatchProviderReadInput {
        optional_params: params,
        ..call
    }
}

fn params_with_request_timeout(request_timeout: f64) -> BatchReadOptionalParams {
    BatchReadOptionalParams {
        request_timeout: Some(request_timeout),
        ..BatchReadOptionalParams::default()
    }
}

fn params_with_timeout(timeout: BatchReadTimeoutValue) -> BatchReadOptionalParams {
    BatchReadOptionalParams {
        timeout: Some(timeout),
        ..BatchReadOptionalParams::default()
    }
}

fn timeout_of(entry: &Recorded) -> Timeout {
    match entry {
        Recorded::OpenAi { timeout, .. }
        | Recorded::Azure { timeout, .. }
        | Recorded::VertexAi { timeout, .. }
        | Recorded::Anthropic { timeout, .. }
        | Recorded::Xai { timeout, .. }
        | Recorded::ProviderConfig { timeout, .. } => timeout.clone(),
        Recorded::BedrockAsyncInvoke { .. } | Recorded::BedrockModelInvocationJob { .. } => {
            panic!("the ARN branches carry no timeout")
        }
    }
}

// the timeout one provider branch received
async fn selected_timeout(provider: &str, params: BatchReadOptionalParams) -> Timeout {
    let (_, calls) = answered(base_settings(), &with_params(input(provider), params)).await;
    timeout_of(&calls[0])
}

// the error the timeout resolution raises before any provider is asked
async fn timeout_error(params: BatchReadOptionalParams) -> BatchProviderReadError<ProviderStatus> {
    let (error, calls) = failed(base_settings(), &with_params(input("openai"), params), 500).await;
    assert!(
        calls.is_empty(),
        "the timeout is rejected before the provider"
    );
    error
}

// the provider argument defaults to openai and every other argument to absent
#[test]
fn missing_arguments_take_their_defaults() {
    let parsed: BatchProviderReadInput =
        serde_json::from_str(r#"{"batch_id": "b"}"#).expect("the batch id alone is enough");
    assert_eq!(parsed.provider, BatchReadProvider::OpenAi);
    assert!(!parsed.aretrieve_batch);
    assert!(parsed.optional_params.api_base.is_none());
    assert!(parsed.optional_params.timeout.is_none());
}

// the else arm that reads AZURE_AD_TOKEN is unreachable, so a token alone does not authenticate
#[tokio::test]
async fn azure_ad_token_setting_does_not_become_the_api_key() {
    let mut resources = resource_sdk_settings::Settings::default();
    resources.azure_ad_token = Some(SecretString::from("ad-token".to_owned()));
    let (_, calls) = answered(configured(resources), &input("azure")).await;
    let Recorded::Azure { api_key, .. } = &calls[0] else {
        panic!("expected an Azure call");
    };
    assert_eq!(api_key.as_deref(), None);
}

#[rstest]
#[case::defaults(BatchReadOptionalParams::default(), Timeout::Seconds(600.0))]
#[case::request_timeout(params_with_request_timeout(5.0), Timeout::Seconds(5.0))]
#[case::optional_timeout_wins(
    BatchReadOptionalParams {
        request_timeout: Some(5.0),
        timeout: Some(BatchReadTimeoutValue::Seconds(7.0)),
        ..BatchReadOptionalParams::default()
    },
    Timeout::Seconds(7.0)
)]
#[case::zero_is_used_as_given(
    params_with_timeout(BatchReadTimeoutValue::Seconds(0.0)),
    Timeout::Seconds(0.0)
)]
#[case::text_number(params_with_timeout(BatchReadTimeoutValue::Text("8.5".to_owned())), Timeout::Seconds(8.5))]
#[case::text_is_trimmed(params_with_timeout(BatchReadTimeoutValue::Text(" 9 ".to_owned())), Timeout::Seconds(9.0))]
#[case::phases_kept(
    params_with_timeout(BatchReadTimeoutValue::Phases(phases(Some(1.0), Some(2.0)))),
    Timeout::Phases(phases(Some(1.0), Some(2.0)))
)]
#[case::phases_without_read_kept(
    params_with_timeout(BatchReadTimeoutValue::Phases(phases(Some(1.0), None))),
    Timeout::Phases(phases(Some(1.0), None))
)]
#[tokio::test]
async fn openai_timeout(#[case] params: BatchReadOptionalParams, #[case] expected: Timeout) {
    assert_eq!(selected_timeout("openai", params).await, expected);
}

// a provider that collapses the pieces reads only read, then the 600 second default
#[rstest]
#[case::read_phase(
    params_with_timeout(BatchReadTimeoutValue::Phases(phases(Some(1.0), Some(4.0)))),
    Timeout::Seconds(4.0)
)]
#[case::missing_read(
    params_with_timeout(BatchReadTimeoutValue::Phases(phases(Some(1.0), None))),
    Timeout::Seconds(600.0)
)]
#[tokio::test]
async fn collapsed_timeout(#[case] params: BatchReadOptionalParams, #[case] expected: Timeout) {
    assert_eq!(selected_timeout("xai", params).await, expected);
}

// only openai, azure and bedrock keep the httpx.Timeout pieces
#[rstest]
#[case::azure("azure", false, Timeout::Phases(phases(Some(1.0), Some(2.0))))]
#[case::bedrock("bedrock", true, Timeout::Phases(phases(Some(1.0), Some(2.0))))]
#[case::mistral("mistral", true, Timeout::Seconds(2.0))]
#[case::xai("xai", false, Timeout::Seconds(2.0))]
#[case::vertex_ai("vertex_ai", false, Timeout::Seconds(2.0))]
#[case::anthropic("anthropic", false, Timeout::Seconds(2.0))]
#[tokio::test]
async fn structured_timeout_per_provider(
    #[case] provider: &str,
    #[case] configured_provider: bool,
    #[case] expected: Timeout,
) {
    let mut call = input(provider);
    if configured_provider {
        call.model = Some("provider/model".to_owned());
    }
    let params = params_with_timeout(BatchReadTimeoutValue::Phases(phases(Some(1.0), Some(2.0))));
    let (_, calls) = answered(base_settings(), &with_params(call, params)).await;
    assert_eq!(timeout_of(&calls[0]), expected);
}

#[rstest]
#[case::text(params_with_timeout(BatchReadTimeoutValue::Text("soon".to_owned())))]
#[case::non_finite_seconds(params_with_timeout(BatchReadTimeoutValue::Seconds(f64::INFINITY)))]
#[case::non_finite_phase(params_with_timeout(BatchReadTimeoutValue::Phases(phases(
    Some(f64::INFINITY),
    Some(1.0)
))))]
#[tokio::test]
async fn invalid_timeout_is_rejected(#[case] params: BatchReadOptionalParams) {
    let error = timeout_error(params).await;
    assert!(matches!(error, BatchProviderReadError::Timeout(_)));
    assert_eq!(error.error_fields(), Some(ReadErrorFields { status: 400 }));
}

// a provider that collapses the phases reads only read, so a bad connect never reaches it
#[tokio::test]
async fn dropped_phase_is_not_validated() {
    let params = params_with_timeout(BatchReadTimeoutValue::Phases(phases(
        Some(f64::INFINITY),
        Some(3.0),
    )));
    assert_eq!(selected_timeout("xai", params).await, Timeout::Seconds(3.0));
}

#[rstest]
#[case::bedrock("bedrock")]
#[case::mistral("mistral")]
#[tokio::test]
async fn configured_provider_retrieve(#[case] provider: &str) {
    let mut call = input(provider);
    call.model = Some("provider/model".to_owned());
    call.aretrieve_batch = true;
    let mut headers = IndexMap::new();
    headers.insert("x-trace".to_owned(), "1".to_owned());
    call.extra_headers = Some(headers);
    call.optional_params.api_base = Some("https://raw.example".to_owned());
    call.optional_params.api_key = Some(SecretString::from("sk-raw".to_owned()));
    call.optional_params.aws_region_name = Some("eu-west-1".to_owned());
    call.optional_params.aws_access_key_id = Some("AKID".to_owned());
    call.optional_params.aws_secret_access_key = Some(SecretString::from("secret".to_owned()));
    call.optional_params.aws_session_token = Some(SecretString::from("token".to_owned()));
    let (_, calls) = answered(base_settings(), &call).await;
    assert_eq!(
        calls,
        vec![Recorded::ProviderConfig {
            provider: BatchReadProvider::from(provider.to_owned()),
            model: "provider/model".to_owned(),
            batch_id: "batch_1".to_owned(),
            headers: BTreeMap::from([("x-trace".to_owned(), "1".to_owned())]),
            api_base: Some("https://raw.example".to_owned()),
            api_key: Some("sk-raw".to_owned()),
            aws: RecordedAws {
                region_name: Some("eu-west-1".to_owned()),
                access_key_id: Some("AKID".to_owned()),
                secret_access_key: Some("secret".to_owned()),
                session_token: Some("token".to_owned()),
            },
            timeout: Timeout::Seconds(600.0),
            is_async: true,
        }]
    );
}

#[tokio::test]
async fn provider_config_without_extras() {
    let mut call = input("bedrock");
    call.model = Some("anthropic.claude".to_owned());
    let (_, calls) = answered(base_settings(), &call).await;
    assert_eq!(
        calls,
        vec![Recorded::ProviderConfig {
            provider: BatchReadProvider::from("bedrock".to_owned()),
            model: "anthropic.claude".to_owned(),
            batch_id: "batch_1".to_owned(),
            headers: BTreeMap::new(),
            api_base: None,
            api_key: None,
            aws: RecordedAws {
                region_name: None,
                access_key_id: None,
                secret_access_key: None,
                session_token: None,
            },
            timeout: Timeout::Seconds(600.0),
            is_async: false,
        }]
    );
}

// a model alone does not give a provider a config
#[tokio::test]
async fn provider_config_needs_a_configured_provider() {
    let mut call = input("openai");
    call.model = Some("gpt-4o".to_owned());
    let (_, calls) = answered(base_settings(), &call).await;
    assert!(matches!(calls[0], Recorded::OpenAi { .. }));
}

#[rstest]
#[case::bedrock_without_model("bedrock")]
#[case::mistral_without_model("mistral")]
#[case::unknown_provider("deepinfra")]
#[tokio::test]
async fn unsupported_provider(#[case] provider: &str) {
    let (error, calls) = failed(base_settings(), &input(provider), 500).await;
    assert!(calls.is_empty(), "no provider is asked");
    let BatchProviderReadError::Unsupported { provider: named } = &error else {
        panic!("an unsupported provider is named");
    };
    assert_eq!(named, provider);
    assert_eq!(error.error_fields(), Some(ReadErrorFields { status: 400 }));
    assert_eq!(
        error.to_string(),
        format!(
            "LiteLLM doesn't support custom_llm_provider={provider} for 'retrieve_batch' without a `model` kwarg. \
             Supported via this path: 'openai', 'azure', 'vertex_ai', 'anthropic'. \
             'bedrock' and 'mistral' are supported but require `model` to be passed so the provider config can be loaded."
        )
    );
}

// LiteLLM builds LlmProviders(custom_llm_provider) only when a model is given, so an unknown name fails there
#[tokio::test]
async fn unknown_provider_with_model_is_a_bad_request() {
    let mut call = input("made_up_provider");
    call.model = Some("some-model".to_owned());
    let (error, calls) = failed(base_settings(), &call, 500).await;
    assert!(calls.is_empty(), "no provider is asked");
    assert!(matches!(error, BatchProviderReadError::Unsupported { .. }));
    assert_eq!(error.error_fields(), Some(ReadErrorFields { status: 400 }));
}

fn arn_input(batch_id: &str, provider: &str) -> BatchProviderReadInput {
    BatchProviderReadInput {
        batch_id: batch_id.to_owned(),
        ..input(provider)
    }
}

fn async_invoke_arn() -> &'static str {
    "arn:aws:bedrock:us-east-1:111122223333:async-invoke/abc"
}

#[rstest]
#[case::default_region(None, Some("us-east-1"))]
#[case::explicit_region(Some("eu-west-1"), Some("eu-west-1"))]
#[tokio::test]
async fn bedrock_async_invoke_region(
    #[case] configured: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let batch_id = async_invoke_arn();
    let mut call = arn_input(batch_id, "openai");
    call.optional_params.aws_region_name = configured.map(str::to_owned);
    let (result, calls) = answered(base_settings(), &call).await;
    assert_eq!(
        calls,
        vec![Recorded::BedrockAsyncInvoke {
            batch_id: batch_id.to_owned(),
            aws: RecordedAws {
                region_name: expected.map(str::to_owned),
                access_key_id: None,
                secret_access_key: None,
                session_token: None,
            },
        }]
    );
    assert!(matches!(
        result.call,
        BatchProviderReadCall::BedrockAsyncInvoke(_)
    ));
}

#[rstest]
#[case::no_region(None, None)]
#[case::configured_region(Some("us-west-2"), Some("us-west-2"))]
#[tokio::test]
async fn bedrock_model_invocation_job_region(
    #[case] configured: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let batch_id = "arn:aws:bedrock:us-east-1:111122223333:model-invocation-job/job-1";
    let mut call = arn_input(batch_id, "openai");
    call.optional_params.aws_region_name = configured.map(str::to_owned);
    call.optional_params.aws_session_token = Some(SecretString::from("token".to_owned()));
    let (_, calls) = answered(base_settings(), &call).await;
    assert_eq!(
        calls,
        vec![Recorded::BedrockModelInvocationJob {
            batch_id: batch_id.to_owned(),
            aws: RecordedAws {
                region_name: expected.map(str::to_owned),
                access_key_id: None,
                secret_access_key: None,
                session_token: Some("token".to_owned()),
            },
        }]
    );
}

// an ARN routes before the provider config, even for a provider that declares one
#[tokio::test]
async fn arn_routing_precedes_the_provider_config() {
    let mut call = arn_input(async_invoke_arn(), "mistral");
    call.model = Some("mistral-large".to_owned());
    let (_, calls) = answered(base_settings(), &call).await;
    assert!(matches!(calls[0], Recorded::BedrockAsyncInvoke { .. }));
}

#[rstest]
#[case::other_bedrock_arn("arn:aws:bedrock:us-east-1:111122223333:some-other/abc")]
#[case::non_bedrock_arn("arn:aws:iam::111122223333:role/job")]
#[case::marker_without_the_arn_prefix("x:bedrock:async-invoke/abc")]
#[case::not_an_arn("batch_1")]
#[tokio::test]
async fn non_routed_arns_reach_the_provider(#[case] batch_id: &str) {
    let (_, calls) = answered(base_settings(), &arn_input(batch_id, "openai")).await;
    assert_eq!(
        calls,
        vec![Recorded::OpenAi {
            request: request(batch_id),
            api_base: "https://api.openai.com/v1".to_owned(),
            api_key: None,
            organization: None,
            timeout: Timeout::Seconds(600.0),
            max_retries: None,
            is_async: false,
        }]
    );
}

#[tokio::test]
async fn extra_headers_and_body_reach_the_request() {
    let mut call = input("azure");
    let mut headers = IndexMap::new();
    headers.insert("a".to_owned(), "b".to_owned());
    let mut body = IndexMap::new();
    body.insert("c".to_owned(), "d".to_owned());
    call.extra_headers = Some(headers.clone());
    call.extra_body = Some(body.clone());
    let (_, calls) = answered(base_settings(), &call).await;
    let Recorded::Azure {
        request: read_request,
        ..
    } = &calls[0]
    else {
        panic!("expected an Azure call");
    };
    assert_eq!(read_request.extra_headers.as_ref(), Some(&headers));
    assert_eq!(read_request.extra_body.as_ref(), Some(&body));
}

#[tokio::test]
async fn openai_reads_the_configured_settings() {
    let mut resources = resource_sdk_settings::Settings::default();
    resources.api_base = Some("https://configured.example".to_owned());
    resources.openai_key = Some(SecretString::from("sk-openai".to_owned()));
    resources.organization = Some("configured-org".to_owned());
    let (_, calls) = answered(configured(resources), &input("openai")).await;
    let Recorded::OpenAi {
        api_base,
        api_key,
        organization,
        ..
    } = &calls[0]
    else {
        panic!("expected an OpenAI-compatible call");
    };
    assert_eq!(api_base, "https://configured.example");
    assert_eq!(api_key.as_deref(), Some("sk-openai"));
    assert_eq!(organization.as_deref(), Some("configured-org"));
}

// the environment fallbacks arrive through the shared settings, not an ambient read here
#[tokio::test]
async fn openai_reads_the_environment_settings() {
    let mut resources = resource_sdk_settings::Settings::default();
    resources.openai_base_url = Some("https://env-base.example".to_owned());
    resources.openai_api_key = Some(SecretString::from("sk-env".to_owned()));
    resources.openai_organization = Some("env-org".to_owned());
    let (_, calls) = answered(configured(resources), &input("openai")).await;
    let Recorded::OpenAi {
        api_base,
        api_key,
        organization,
        ..
    } = &calls[0]
    else {
        panic!("expected an OpenAI-compatible call");
    };
    assert_eq!(api_base, "https://env-base.example");
    assert_eq!(api_key.as_deref(), Some("sk-env"));
    assert_eq!(organization.as_deref(), Some("env-org"));
}

#[tokio::test]
async fn openai_prefers_the_request_parameters() {
    let mut resources = resource_sdk_settings::Settings::default();
    resources.api_base = Some("https://configured.example".to_owned());
    resources.openai_key = Some(SecretString::from("sk-openai".to_owned()));
    resources.organization = Some("configured-org".to_owned());
    let params = BatchReadOptionalParams {
        api_base: Some("https://request.example".to_owned()),
        api_key: Some(SecretString::from("sk-request".to_owned())),
        organization: Some("request-org".to_owned()),
        ..BatchReadOptionalParams::default()
    };
    let call = with_params(input("openai"), params);
    let (_, calls) = answered(configured(resources), &call).await;
    let Recorded::OpenAi {
        api_base,
        api_key,
        organization,
        ..
    } = &calls[0]
    else {
        panic!("expected an OpenAI-compatible call");
    };
    assert_eq!(api_base, "https://request.example");
    assert_eq!(api_key.as_deref(), Some("sk-request"));
    assert_eq!(organization.as_deref(), Some("request-org"));
}

#[tokio::test]
async fn azure_reads_the_configured_and_environment_settings() {
    let mut resources = resource_sdk_settings::Settings::default();
    resources.azure_api_base = Some("https://azure-env.example".to_owned());
    resources.azure_api_version = Some("2024-06-01".to_owned());
    resources.azure_openai_api_key = Some(SecretString::from("azure-env-key".to_owned()));
    let (_, calls) = answered(configured(resources), &input("azure")).await;
    let Recorded::Azure {
        api_base,
        api_key,
        api_version,
        litellm_params,
        ..
    } = &calls[0]
    else {
        panic!("expected an Azure call");
    };
    assert_eq!(api_base.as_deref(), Some("https://azure-env.example"));
    assert_eq!(api_key.as_deref(), Some("azure-env-key"));
    assert_eq!(api_version.as_deref(), Some("2024-06-01"));
    assert_eq!(litellm_params.api_key, None);
    assert_eq!(litellm_params.api_base, None);
    assert_eq!(litellm_params.api_version, None);
}

fn raw_azure_params() -> BatchReadOptionalParams {
    BatchReadOptionalParams {
        api_key: Some(SecretString::from("sk-raw".to_owned())),
        api_base: Some("https://raw.example".to_owned()),
        api_version: Some("raw-version".to_owned()),
        max_retries: Some(2),
        timeout: Some(BatchReadTimeoutValue::Seconds(3.0)),
        azure_ad_token: Some(SecretString::from("ad-token".to_owned())),
        tenant_id: Some("tenant".to_owned()),
        client_id: Some("client".to_owned()),
        client_secret: Some(SecretString::from("client-secret".to_owned())),
        azure_scope: Some("scope".to_owned()),
        azure_username: Some("user".to_owned()),
        azure_password: Some(SecretString::from("password".to_owned())),
        ..BatchReadOptionalParams::default()
    }
}

// the same values, unresolved, as the azure call record carries them
fn raw_recorded_params() -> RecordedParams {
    RecordedParams {
        api_key: Some("sk-raw".to_owned()),
        api_base: Some("https://raw.example".to_owned()),
        api_version: Some("raw-version".to_owned()),
        max_retries: Some(2),
        timeout: Some(BatchReadTimeoutValue::Seconds(3.0)),
        azure_ad_token: Some("ad-token".to_owned()),
        tenant_id: Some("tenant".to_owned()),
        client_id: Some("client".to_owned()),
        client_secret: Some("client-secret".to_owned()),
        azure_scope: Some("scope".to_owned()),
        azure_username: Some("user".to_owned()),
        azure_password: Some("password".to_owned()),
    }
}

// the azure client reads the raw kwargs, so the request values stay unresolved inside litellm_params
#[tokio::test]
async fn azure_forwards_the_raw_kwargs() {
    let mut resources = resource_sdk_settings::Settings::default();
    resources.azure_api_base = Some("https://azure-env.example".to_owned());
    let call = with_params(input("azure"), raw_azure_params());
    let (_, calls) = answered(configured(resources), &call).await;
    let Recorded::Azure {
        api_base,
        api_key,
        litellm_params,
        max_retries,
        timeout,
        ..
    } = &calls[0]
    else {
        panic!("expected an Azure call");
    };
    assert_eq!(api_base.as_deref(), Some("https://raw.example"));
    assert_eq!(api_key.as_deref(), Some("sk-raw"));
    assert_eq!(*max_retries, Some(2));
    assert_eq!(timeout, &Timeout::Seconds(3.0));
    assert_eq!(litellm_params, &raw_recorded_params());
}

async fn vertex_credentials_of(
    credentials: Option<Json>,
    resources: resource_sdk_settings::Settings,
) -> Option<VertexCredentials> {
    let params = BatchReadOptionalParams {
        vertex_credentials: credentials,
        ..BatchReadOptionalParams::default()
    };
    let call = with_params(input("vertex_ai"), params);
    let (_, calls) = answered(configured(resources), &call).await;
    let Recorded::VertexAi {
        vertex_credentials, ..
    } = &calls[0]
    else {
        panic!("expected a Vertex call");
    };
    vertex_credentials.clone()
}

#[tokio::test]
async fn vertex_credentials_accept_an_inline_account() {
    let Json::Object(fields) = json_value(r#"{"type": "service_account"}"#) else {
        panic!("an object");
    };
    assert_eq!(
        vertex_credentials_of(
            Some(Json::Object(fields.clone())),
            resource_sdk_settings::Settings::default()
        )
        .await,
        Some(VertexCredentials::ServiceAccount(fields))
    );
}

#[rstest]
#[case::request_path(Some(json_value("\"request-ca.json\"")), "request-ca.json")]
#[case::json_null(Some(json_value("null")), "env-ca.json")]
#[case::absent(None, "env-ca.json")]
#[tokio::test]
async fn vertex_credentials_read_the_shared_chain(
    #[case] credentials: Option<Json>,
    #[case] expected: &str,
) {
    let mut resources = resource_sdk_settings::Settings::default();
    resources.vertexai_credentials = Some(SecretString::from("env-ca.json".to_owned()));
    assert_eq!(
        vertex_credentials_of(credentials, resources).await,
        Some(VertexCredentials::Path(expected.to_owned()))
    );
}

#[rstest]
#[case::bool(json_value("true"))]
#[case::number(json_value("1"))]
#[case::list(json_value("[]"))]
#[tokio::test]
async fn vertex_credentials_reject_other_types(#[case] credentials: Json) {
    let params = BatchReadOptionalParams {
        vertex_credentials: Some(credentials),
        ..BatchReadOptionalParams::default()
    };
    let (error, calls) = failed(
        base_settings(),
        &with_params(input("vertex_ai"), params),
        500,
    )
    .await;
    assert!(calls.is_empty(), "no provider is asked");
    assert!(matches!(
        error,
        BatchProviderReadError::InvalidVertexCredentials
    ));
    assert_eq!(error.error_fields(), Some(ReadErrorFields { status: 400 }));
}

#[tokio::test]
async fn vertex_reads_the_configured_settings() {
    let mut resources = resource_sdk_settings::Settings::default();
    resources.vertex_project = Some("configured-project".to_owned());
    resources.vertexai_location = Some("europe-west4".to_owned());
    let (_, calls) = answered(configured(resources), &input("vertex_ai")).await;
    let Recorded::VertexAi {
        vertex_project,
        vertex_location,
        ..
    } = &calls[0]
    else {
        panic!("expected a Vertex call");
    };
    assert_eq!(vertex_project.as_deref(), Some("configured-project"));
    assert_eq!(vertex_location.as_deref(), Some("europe-west4"));
}

#[tokio::test]
async fn anthropic_reads_the_shared_settings() {
    let mut resources = resource_sdk_settings::Settings::default();
    resources.api_base = Some("https://shared.example".to_owned());
    resources.api_key = Some(SecretString::from("sk-shared".to_owned()));
    let (_, calls) = answered(configured(resources), &input("anthropic")).await;
    let Recorded::Anthropic {
        api_base, api_key, ..
    } = &calls[0]
    else {
        panic!("expected an Anthropic call");
    };
    assert_eq!(api_base.as_deref(), Some("https://shared.example"));
    assert_eq!(api_key.as_deref(), Some("sk-shared"));
}

#[rstest]
#[case::api_base(
    Some("https://api-base.example"),
    Some("https://base-url.example"),
    "https://api-base.example"
)]
#[case::base_url(None, Some("https://base-url.example"), "https://base-url.example")]
#[tokio::test]
async fn anthropic_reads_its_own_environment(
    #[case] api_base: Option<&str>,
    #[case] base_url: Option<&str>,
    #[case] expected: &str,
) {
    let settings = Settings {
        anthropic_api_base: api_base.map(str::to_owned),
        anthropic_base_url: base_url.map(str::to_owned),
        anthropic_api_key: Some(SecretString::from("sk-anthropic".to_owned())),
        ..Settings::default()
    };
    let (_, calls) = answered(settings, &input("anthropic")).await;
    let Recorded::Anthropic {
        api_base: recorded_base,
        api_key,
        ..
    } = &calls[0]
    else {
        panic!("expected an Anthropic call");
    };
    assert_eq!(recorded_base.as_deref(), Some(expected));
    assert_eq!(api_key.as_deref(), Some("sk-anthropic"));
}

// the branch falls back through litellm.azure_key, which the pinned source keeps
#[tokio::test]
async fn anthropic_keeps_the_azure_key_fallback() {
    let mut resources = resource_sdk_settings::Settings::default();
    resources.azure_key = Some(SecretString::from("azure-configured-key".to_owned()));
    let (_, calls) = answered(configured(resources), &input("anthropic")).await;
    let Recorded::Anthropic { api_key, .. } = &calls[0] else {
        panic!("expected an Anthropic call");
    };
    assert_eq!(api_key.as_deref(), Some("azure-configured-key"));
}

#[tokio::test]
async fn extra_entries_over_the_limit_are_rejected() {
    let mut call = input("openai");
    let mut headers = IndexMap::new();
    headers.insert("a".to_owned(), "1".to_owned());
    headers.insert("b".to_owned(), "2".to_owned());
    call.extra_headers = Some(headers);
    let (dispatch, calls) = scripted(Ok(answer(BatchStatus::Completed)));
    let service = BatchProviderRead {
        dispatch,
        limits: ApiLimits {
            max_items: 1,
            ..ApiLimits::default()
        },
        settings: base_settings(),
    };
    let error = service.run(&call).await.expect_err("the limit rejects");
    assert!(matches!(
        error,
        BatchProviderReadError::Limits(ApiLimitError::TooManyItems {
            observed: 2,
            limit: 1
        })
    ));
    assert!(recorded_calls(&calls).is_empty());
}

// the batch id and every extra header and body entry count toward the request size
#[tokio::test]
async fn an_oversized_request_is_rejected() {
    let mut call = input("openai");
    let mut headers = IndexMap::new();
    headers.insert("a".to_owned(), "1".to_owned());
    call.extra_headers = Some(headers);
    let (dispatch, calls) = scripted(Ok(answer(BatchStatus::Completed)));
    let service = BatchProviderRead {
        dispatch,
        limits: ApiLimits {
            max_json_bytes: 8,
            ..ApiLimits::default()
        },
        settings: base_settings(),
    };
    let error = service.run(&call).await.expect_err("the limit rejects");
    assert!(matches!(
        error,
        BatchProviderReadError::Limits(ApiLimitError::JsonTooLarge {
            observed: 9,
            limit: 8
        })
    ));
    assert!(recorded_calls(&calls).is_empty());
}
