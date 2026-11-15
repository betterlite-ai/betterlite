use std::sync::{Arc, Mutex};

use betterlite_openai::batches::records::BatchStatus;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use crate::api::api_boundaries::ApiLimits;
use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutValue,
};

use super::super::connection::{self, CancelTimeout};
use super::super::{
    BatchProviderCancelError, BatchProviderCancelInput, CancelBatchCall, CancelCall, CancelProvider,
};
use super::{
    BatchProviderCancel, Recorded, SECOND, ScriptFailure, Scripted, answer, base_params,
    base_settings, first_call, recorded_calls, request, scripted, service, values,
};

// the structured timeout the source keeps as httpx.Timeout on the providers that declare it
const STRUCTURED: StructuredTimeout = StructuredTimeout {
    connect: Some(1.0_f64),
    read: Some(30.0_f64),
    write: Some(2.0_f64),
    pool: Some(3.0_f64),
};

fn structured(connect: f64, read: f64, write: f64, pool: f64) -> CancelTimeout {
    CancelTimeout::Structured(StructuredTimeout {
        connect: Some(connect),
        read: Some(read),
        write: Some(write),
        pool: Some(pool),
    })
}

#[tokio::test]
async fn batch_cancel_forwards_the_raw_parameters_to_bedrock() {
    let mut params = base_params();
    params.aws_region_name = Some("us-east-1".to_owned());
    params.aws_access_key_id = Some("access".to_owned());
    params.aws_session_token = Some(SecretString::from("session".to_owned()));
    let Recorded::Bedrock {
        values,
        aws_region_name,
        aws_access_key_id,
        aws_session_token,
    } = first_call(base_settings(), "bedrock", params).await
    else {
        panic!("bedrock takes its branch");
    };
    assert_eq!(values.batch_id, "batch_1");
    assert_eq!(aws_region_name.as_deref(), Some("us-east-1"));
    assert_eq!(aws_access_key_id.as_deref(), Some("access"));
    assert_eq!(aws_session_token.as_deref(), Some("session"));
}

#[tokio::test]
async fn batch_cancel_prefers_call_parameters_over_settings() {
    let mut settings = base_settings();
    settings.api_base = Some("https://settings.example".to_owned());
    settings.organization = Some("org-settings".to_owned());
    settings.api_key = Some(SecretString::from("sk-settings".to_owned()));
    let mut params = base_params();
    params.api_base = Some("https://param.example".to_owned());
    params.organization = Some("org-param".to_owned());
    params.api_key = Some(SecretString::from("sk-param".to_owned()));
    let Recorded::OpenAi {
        api_base,
        api_key,
        organization,
        ..
    } = first_call(settings, "openai", params).await
    else {
        panic!("openai takes its branch");
    };
    assert_eq!(api_base, "https://param.example");
    assert_eq!(organization.as_deref(), Some("org-param"));
    assert_eq!(api_key.as_deref(), Some("sk-param"));
}

#[tokio::test]
async fn batch_cancel_forwards_the_extra_headers_body_and_retries() {
    let mut params = base_params();
    params.max_retries = Some(2);
    let input = BatchProviderCancelInput::Cancel(Box::new(CancelBatchCall {
        batch_id: "batch_1".to_owned(),
        model: None,
        provider: CancelProvider("openai".to_owned()),
        extra_headers: Some(IndexMap::from([("x-key".to_owned(), "v".to_owned())])),
        extra_body: Some(IndexMap::from([("custom".to_owned(), "value".to_owned())])),
        params,
        call: CancelCall::Sync,
    }));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(base_settings(), dispatch);
    service.run(&input).await.expect("answered");
    let entries = recorded_calls(&calls);
    let forwarded = values(&entries[0]);
    assert_eq!(
        forwarded.request.extra_headers,
        Some(IndexMap::from([("x-key".to_owned(), "v".to_owned())]))
    );
    assert_eq!(
        forwarded.request.extra_body,
        Some(IndexMap::from([("custom".to_owned(), "value".to_owned())]))
    );
    assert_eq!(forwarded.request.timeout, None);
    assert_eq!(forwarded.max_retries, Some(2));
}

#[tokio::test]
async fn batch_cancel_reads_the_organization_setting() {
    let mut settings = base_settings();
    settings.organization = Some("org-settings".to_owned());
    settings.openai_organization = Some("org-env".to_owned());
    let Recorded::OpenAi { organization, .. } = first_call(settings, "openai", base_params()).await
    else {
        panic!("openai takes its branch");
    };
    assert_eq!(organization.as_deref(), Some("org-settings"));
}

#[tokio::test]
async fn batch_cancel_reads_the_openai_organization_env() {
    let mut settings = base_settings();
    settings.openai_organization = Some("org-env".to_owned());
    let Recorded::OpenAi { organization, .. } = first_call(settings, "openai", base_params()).await
    else {
        panic!("openai takes its branch");
    };
    assert_eq!(organization.as_deref(), Some("org-env"));
}

#[tokio::test]
async fn batch_cancel_reads_configured_settings() {
    let mut settings = base_settings();
    settings.api_base = Some("https://settings.example".to_owned());
    settings.openai_key = Some(SecretString::from("sk-openai".to_owned()));
    let Recorded::OpenAi {
        api_base, api_key, ..
    } = first_call(settings, "openai", base_params()).await
    else {
        panic!("openai takes its branch");
    };
    assert_eq!(api_base, "https://settings.example");
    assert_eq!(api_key.as_deref(), Some("sk-openai"));
}

#[rstest]
#[case::openai_base_url(
    None,
    Some("https://base-url.example"),
    None,
    "https://base-url.example"
)]
#[case::openai_api_base(
    None,
    None,
    Some("https://api-base.example"),
    "https://api-base.example"
)]
#[case::base_url_wins(
    Some("https://first.example"),
    Some("https://base-url.example"),
    None,
    "https://first.example"
)]
#[tokio::test]
async fn batch_cancel_environment_fallbacks(
    #[case] base: Option<&str>,
    #[case] base_url: Option<&str>,
    #[case] openai_api_base: Option<&str>,
    #[case] expected: &str,
) {
    let mut settings = base_settings();
    settings.api_base = base.map(str::to_owned);
    settings.openai_base_url = base_url.map(str::to_owned);
    settings.openai_api_base = openai_api_base.map(str::to_owned);
    let Recorded::OpenAi { api_base, .. } = first_call(settings, "openai", base_params()).await
    else {
        panic!("openai takes its branch");
    };
    assert_eq!(api_base, expected);
}

#[tokio::test]
async fn batch_cancel_skips_empty_bases_and_credentials() {
    let mut settings = base_settings();
    settings.api_base = Some(String::new());
    settings.api_key = Some(SecretString::from(String::new()));
    settings.openai_key = Some(SecretString::from(String::new()));
    settings.openai_api_key = Some(SecretString::from("sk-openai-api".to_owned()));
    let Recorded::OpenAi {
        api_base, api_key, ..
    } = first_call(settings, "openai", base_params()).await
    else {
        panic!("openai takes its branch");
    };
    assert_eq!(api_base, "https://api.openai.com/v1");
    assert_eq!(api_key.as_deref(), Some("sk-openai-api"));
}

#[tokio::test]
async fn batch_cancel_resolves_azure_settings() {
    let mut settings = base_settings();
    settings.azure_api_base = Some("https://azure.example".to_owned());
    settings.azure_api_version = Some("2024-06-01".to_owned());
    settings.azure_key = Some(SecretString::from("azure-key".to_owned()));
    let Recorded::Azure {
        api_base,
        api_key,
        api_version,
        ..
    } = first_call(settings, "azure", base_params()).await
    else {
        panic!("azure takes its branch");
    };
    assert_eq!(api_base.as_deref(), Some("https://azure.example"));
    assert_eq!(api_version.as_deref(), Some("2024-06-01"));
    assert_eq!(api_key.as_deref(), Some("azure-key"));
}

#[tokio::test]
async fn batch_cancel_leaves_unset_settings_empty() {
    let Recorded::Azure {
        api_base: azure_base,
        api_key: azure_key,
        api_version,
        azure_ad_token,
        ..
    } = first_call(base_settings(), "azure", base_params()).await
    else {
        panic!("azure takes its branch");
    };
    assert_eq!(azure_base, None);
    assert_eq!(azure_key, None);
    assert_eq!(api_version, None);
    assert_eq!(azure_ad_token, None);
    let Recorded::Vertex {
        api_base: vertex_base,
        vertex_project,
        vertex_location,
        vertex_credentials,
        ..
    } = first_call(base_settings(), "vertex_ai", base_params()).await
    else {
        panic!("vertex takes its branch");
    };
    assert_eq!(vertex_base, None);
    assert_eq!(vertex_project, None);
    assert_eq!(vertex_location, None);
    assert_eq!(vertex_credentials, None);
    let Recorded::Xai {
        api_base: xai_base,
        api_key: xai_key,
        ..
    } = first_call(base_settings(), "xai", base_params()).await
    else {
        panic!("xai takes its branch");
    };
    assert_eq!((xai_base, xai_key), (None, None));
}

#[tokio::test]
async fn batch_cancel_reads_the_azure_openai_key() {
    let mut settings = base_settings();
    settings.azure_openai_api_key = Some(SecretString::from("azure-openai-key".to_owned()));
    let Recorded::Azure { api_key, .. } = first_call(settings, "azure", base_params()).await else {
        panic!("azure takes its branch");
    };
    assert_eq!(api_key.as_deref(), Some("azure-openai-key"));
}

#[tokio::test]
async fn batch_cancel_reads_the_azure_api_key() {
    let mut settings = base_settings();
    settings.azure_api_key = Some(SecretString::from("azure-api-key".to_owned()));
    let Recorded::Azure { api_key, .. } = first_call(settings, "azure", base_params()).await else {
        panic!("azure takes its branch");
    };
    assert_eq!(api_key.as_deref(), Some("azure-api-key"));
}

#[tokio::test]
async fn batch_cancel_resolves_the_azure_ad_token() {
    let mut settings = base_settings();
    settings.azure_ad_token = Some(SecretString::from("env-token".to_owned()));
    let mut params = base_params();
    params.azure_ad_token = Some(SecretString::from("param-token".to_owned()));
    let Recorded::Azure {
        azure_ad_token: param_token,
        ..
    } = first_call(settings.clone(), "azure", params).await
    else {
        panic!("azure takes its branch");
    };
    assert_eq!(param_token.as_deref(), Some("param-token"));
    let mut empty = base_params();
    empty.azure_ad_token = Some(SecretString::from(String::new()));
    let Recorded::Azure {
        azure_ad_token: env_token,
        ..
    } = first_call(settings, "azure", empty).await
    else {
        panic!("azure takes its branch");
    };
    assert_eq!(env_token.as_deref(), Some("env-token"));
}

#[tokio::test]
async fn batch_cancel_resolves_vertex_settings() {
    let mut settings = base_settings();
    settings.vertex_project = Some("project-settings".to_owned());
    settings.vertexai_project = Some("project-env".to_owned());
    settings.vertexai_location = Some("location-env".to_owned());
    settings.vertexai_credentials = Some(SecretString::from("vertex-token".to_owned()));
    let Recorded::Vertex {
        vertex_project,
        vertex_location,
        vertex_credentials,
        ..
    } = first_call(settings, "vertex_ai", base_params()).await
    else {
        panic!("vertex takes its branch");
    };
    assert_eq!(vertex_project.as_deref(), Some("project-settings"));
    assert_eq!(vertex_location.as_deref(), Some("location-env"));
    assert_eq!(vertex_credentials.as_deref(), Some("vertex-token"));
}

#[tokio::test]
async fn batch_cancel_reads_the_vertex_location_setting() {
    let mut settings = base_settings();
    settings.vertex_location = Some("location-settings".to_owned());
    settings.vertexai_location = Some("location-env".to_owned());
    let Recorded::Vertex {
        vertex_location, ..
    } = first_call(settings, "vertex_ai", base_params()).await
    else {
        panic!("vertex takes its branch");
    };
    assert_eq!(vertex_location.as_deref(), Some("location-settings"));
}

#[tokio::test]
async fn batch_cancel_reads_the_vertex_location_setting_alone() {
    let mut settings = base_settings();
    settings.vertex_location = Some("location-settings".to_owned());
    let Recorded::Vertex {
        vertex_location, ..
    } = first_call(settings, "vertex_ai", base_params()).await
    else {
        panic!("vertex takes its branch");
    };
    assert_eq!(vertex_location.as_deref(), Some("location-settings"));
}

#[tokio::test]
async fn batch_cancel_reads_the_vertex_project_environment() {
    let mut settings = base_settings();
    settings.vertexai_project = Some("project-env".to_owned());
    let Recorded::Vertex { vertex_project, .. } =
        first_call(settings, "vertex_ai", base_params()).await
    else {
        panic!("vertex takes its branch");
    };
    assert_eq!(vertex_project.as_deref(), Some("project-env"));
}

#[tokio::test]
async fn batch_cancel_xai_reads_only_the_call_parameters() {
    let mut settings = base_settings();
    settings.api_base = Some("https://settings.example".to_owned());
    settings.openai_key = Some(SecretString::from("sk-openai".to_owned()));
    let Recorded::Xai {
        api_base, api_key, ..
    } = first_call(settings, "xai", base_params()).await
    else {
        panic!("xai takes its branch");
    };
    assert_eq!(api_base, None);
    assert_eq!(api_key, None);
}

#[rstest]
#[case::default_timeout(None, None, FineTuningTimeout::Seconds(600.0))]
#[case::request_timeout(None, Some(42.5_f64), FineTuningTimeout::Seconds(42.5))]
#[case::configured_timeout(
    Some(TimeoutValue::Number(12.0)),
    Some(42.5_f64),
    FineTuningTimeout::Seconds(12.0)
)]
#[case::text_timeout(Some(TimeoutValue::String(" 7.5 ".to_owned())), None, FineTuningTimeout::Seconds(7.5))]
#[case::underscore_timeout(Some(TimeoutValue::String("1_000".to_owned())), None, FineTuningTimeout::Seconds(1000.0))]
#[case::zero_timeout_falls_back(
    Some(TimeoutValue::Number(0.0)),
    Some(42.5_f64),
    FineTuningTimeout::Seconds(42.5)
)]
#[case::empty_text_falls_back(
    Some(TimeoutValue::String(String::new())),
    Some(42.5_f64),
    FineTuningTimeout::Seconds(42.5)
)]
#[case::zero_request_timeout(None, Some(0.0_f64), FineTuningTimeout::Seconds(600.0))]
#[tokio::test]
async fn batch_cancel_normalizes_the_timeout(
    #[case] timeout: Option<TimeoutValue>,
    #[case] request_timeout: Option<f64>,
    #[case] expected: FineTuningTimeout,
) {
    let mut params = base_params();
    params.timeout = timeout.map(CancelTimeout::Plain);
    params.request_timeout = request_timeout;
    let entry = first_call(base_settings(), "openai", params).await;
    assert_eq!(values(&entry).timeout, expected);
}

#[rstest]
#[case::openai("openai", FineTuningTimeout::Structured(STRUCTURED))]
#[case::azure("azure", FineTuningTimeout::Structured(STRUCTURED))]
#[case::vertex_ai("vertex_ai", FineTuningTimeout::Seconds(30.0))]
#[case::xai("xai", FineTuningTimeout::Seconds(30.0))]
#[case::hosted_vllm("hosted_vllm", FineTuningTimeout::Seconds(30.0))]
#[case::litellm_proxy("litellm_proxy", FineTuningTimeout::Seconds(30.0))]
#[tokio::test]
async fn batch_cancel_resolves_a_structured_timeout(
    #[case] provider: &str,
    #[case] expected: FineTuningTimeout,
) {
    let mut params = base_params();
    params.timeout = Some(structured(1.0, 30.0, 2.0, 3.0));
    let entry = first_call(base_settings(), provider, params).await;
    assert_eq!(values(&entry).timeout, expected);
}

#[test]
fn batch_cancel_keeps_a_structured_timeout_for_bedrock() {
    let mut params = base_params();
    params.timeout = Some(structured(1.0, 30.0, 2.0, 3.0));
    let BatchProviderCancelInput::Cancel(call) = request("bedrock", params, CancelCall::Sync)
    else {
        panic!("the fixture names a cancel call");
    };
    assert_eq!(
        connection::resolve_timeout(&call).expect("resolved"),
        FineTuningTimeout::Structured(STRUCTURED)
    );
}

#[tokio::test]
async fn batch_cancel_structured_timeout_without_read_falls_back() {
    let mut params = base_params();
    params.timeout = Some(CancelTimeout::Structured(StructuredTimeout {
        connect: None,
        read: None,
        write: None,
        pool: None,
    }));
    let entry = first_call(base_settings(), "vertex_ai", params).await;
    assert_eq!(values(&entry).timeout, FineTuningTimeout::Seconds(600.0));
}

#[tokio::test]
async fn batch_cancel_rejects_a_timeout_that_is_not_a_number() {
    let mut params = base_params();
    params.timeout = Some(CancelTimeout::Plain(TimeoutValue::String(
        "later".to_owned(),
    )));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(base_settings(), dispatch);
    let error = service
        .run(&request("openai", params, CancelCall::Sync))
        .await
        .expect_err("the timeout is not a number");
    assert!(matches!(&error, BatchProviderCancelError::Timeout(_)));
    assert!(error.to_string().contains("timeout"), "{error}");
    assert!(recorded_calls(&calls).is_empty());
}

#[rstest]
#[case::sync_uses_the_argument(CancelCall::Sync, Some("gpt-4o"), Some("gpt-4o"), false)]
#[case::async_keeps_the_argument(CancelCall::Async { model_in_kwargs: None }, Some("gpt-4o"), Some("gpt-4o"), true)]
#[case::async_kwargs_model_wins(CancelCall::Async { model_in_kwargs: Some("gpt-4o-mini".to_owned()) }, Some("gpt-4o"), Some("gpt-4o-mini"), true)]
#[case::async_without_a_model(CancelCall::Async { model_in_kwargs: None }, None, None, true)]
#[tokio::test]
async fn batch_cancel_merges_the_model(
    #[case] call: CancelCall,
    #[case] model: Option<&str>,
    #[case] expected: Option<&str>,
    #[case] is_async: bool,
) {
    let input = BatchProviderCancelInput::Cancel(Box::new(CancelBatchCall {
        batch_id: "batch_1".to_owned(),
        model: model.map(str::to_owned),
        provider: CancelProvider("openai".to_owned()),
        extra_headers: None,
        extra_body: None,
        params: base_params(),
        call,
    }));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(base_settings(), dispatch);
    let result = service.run(&input).await.expect("answered");
    assert_eq!(result.model.as_deref(), expected);
    assert_eq!(values(&recorded_calls(&calls)[0]).is_async, is_async);
}

#[tokio::test]
async fn batch_cancel_rejects_an_unsupported_provider_with_400() {
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(base_settings(), dispatch);
    let error = service
        .run(&request("mistral", base_params(), CancelCall::Sync))
        .await
        .expect_err("the provider is unsupported");
    let fields = error.error_fields().expect("an HTTP error");
    assert_eq!(fields.status, 400);
    assert_eq!(fields.error_type, None);
    assert_eq!(fields.param, None);
    assert_eq!(fields.code, None);
    assert!(error.to_string().contains("mistral"), "{error}");
    assert!(recorded_calls(&calls).is_empty());
}

#[rstest]
#[case::cancelling(BatchStatus::Cancelling)]
#[case::cancelled(BatchStatus::Cancelled)]
#[case::completed(BatchStatus::Completed)]
#[tokio::test]
async fn batch_cancel_preserves_the_provider_status(#[case] status: BatchStatus) {
    let (dispatch, _calls) = scripted(status);
    let service = service(base_settings(), dispatch);
    let result = service
        .run(&request("openai", base_params(), CancelCall::Sync))
        .await
        .expect("answered");
    assert_eq!(result.batch, answer(status));
    assert_eq!(result.model, None);
}

#[tokio::test]
async fn batch_cancel_surfaces_a_provider_error() {
    let service = service(
        base_settings(),
        Scripted {
            calls: Arc::new(Mutex::new(Vec::new())),
            reply: Err(ScriptFailure {
                message: "the provider refused".to_owned(),
            }),
        },
    );
    let error = service
        .run(&request("openai", base_params(), CancelCall::Sync))
        .await
        .expect_err("the provider refused");
    assert!(matches!(&error, BatchProviderCancelError::Dispatch(_)));
    assert_eq!(error.to_string(), "the provider refused");
}

#[rstest]
#[case::at_the_limit(Some(("a", "bc")), None, false)]
#[case::headers_over_the_limit(Some(("ab", "cd")), None, true)]
#[case::body_over_the_limit(None, Some(("ab", "cd")), true)]
#[case::both_over_the_limit(Some(("a", "bc")), Some(("d", "e")), true)]
#[tokio::test]
async fn batch_cancel_bounds_the_extra_bytes(
    #[case] header: Option<(&str, &str)>,
    #[case] body: Option<(&str, &str)>,
    #[case] rejected: bool,
) {
    let defaults = ApiLimits::default();
    let limits = ApiLimits {
        max_json_bytes: 10,
        max_event_bytes: defaults.max_event_bytes,
        max_items: defaults.max_items,
    };
    let entry = |pair: (&str, &str)| IndexMap::from([(pair.0.to_owned(), pair.1.to_owned())]);
    let input = BatchProviderCancelInput::Cancel(Box::new(CancelBatchCall {
        batch_id: "batch_1".to_owned(),
        model: None,
        provider: CancelProvider("openai".to_owned()),
        extra_headers: header.map(entry),
        extra_body: body.map(entry),
        params: base_params(),
        call: CancelCall::Sync,
    }));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = BatchProviderCancel {
        clock: Arc::new(|| SECOND),
        dispatch,
        limits,
        settings: base_settings(),
    };
    let outcome = service.run(&input).await;
    assert_eq!(
        matches!(outcome, Err(BatchProviderCancelError::Limits(_))),
        rejected
    );
    assert_eq!(recorded_calls(&calls).is_empty(), rejected);
}

#[tokio::test]
async fn batch_cancel_bounds_the_extra_entries() {
    let limit = ApiLimits::default().max_items;
    let extra_headers: IndexMap<String, String> = (0..=limit)
        .map(|index| (index.to_string(), String::new()))
        .collect();
    let input = BatchProviderCancelInput::Cancel(Box::new(CancelBatchCall {
        batch_id: "batch_1".to_owned(),
        model: None,
        provider: CancelProvider("openai".to_owned()),
        extra_headers: Some(extra_headers),
        extra_body: None,
        params: base_params(),
        call: CancelCall::Sync,
    }));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(base_settings(), dispatch);
    let error = service.run(&input).await.expect_err("too many entries");
    assert!(matches!(error, BatchProviderCancelError::Limits(_)));
    assert!(recorded_calls(&calls).is_empty());
}
