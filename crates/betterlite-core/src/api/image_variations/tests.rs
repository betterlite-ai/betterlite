#![cfg(test)]

mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::image_contracts::records::{ImageData, ImageResponse};
use http::Method;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use super::call::{DEFAULT_MODEL, VariationAnswer, VariationOptions, VariationResponseFormat};
use super::credentials::{DEFAULT_OPENAI_API_BASE, DEFAULT_TOPAZ_API_BASE, VariationPolicies};
use super::entry::{aimage_variation, image_variation};
use super::error::ImageVariationError;

use self::support::{
    ERROR_BODY, FailingOperation, Fixture, NoTopaz, RecordingTopaz, RecordingTransport, SOURCE,
    SUCCESS_BODY, Scripted, ScriptedOperation, ignored_options, metadata, multipart_parts,
    recorded_calls, response, service, settings, variation_call,
};

#[test]
fn signature_defaults_match_litellm() {
    let options = VariationOptions::default();
    assert_eq!(DEFAULT_MODEL, "dall-e-2");
    assert_eq!(options.n, 1);
    assert_eq!(options.response_format, VariationResponseFormat::Url);
    assert_eq!(options.size, None);
    assert_eq!(options.user, None);
}

#[tokio::test]
async fn openai_variation_posts_the_image_alone() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("OPENAI_API_KEY", "sk-test")]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let service = service(
        &settings,
        &context,
        &transport,
        &NoTopaz,
        VariationPolicies::default(),
    );
    let mut call = variation_call();
    call.model = Some("openai/dall-e-2".to_owned());
    call.options = ignored_options();
    let answer = image_variation(&service, &call)
        .await
        .expect("the variation succeeds");
    let requests = transport.take();
    let [request] = requests.as_slice() else {
        panic!("exactly one variation request");
    };
    assert_eq!(request.method, Method::POST);
    assert_eq!(
        request.url.as_str(),
        "https://api.openai.com/v1/images/variations"
    );
    assert_eq!(request.headers["authorization"], "Bearer sk-test");
    let [part] = multipart_parts(request) else {
        panic!("the image is the only part");
    };
    assert_eq!(part.name, "image");
    assert_eq!(part.file_name.as_deref(), Some("a.png"));
    assert_eq!(part.content_type.as_deref(), Some("image/png"));
    assert_eq!(part.data, b"abc");
    assert_eq!(answer, VariationAnswer::Response(response()));
}

#[tokio::test]
async fn async_variation_sends_the_same_request() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("OPENAI_API_KEY", "sk-test")]);
    let sync_transport = RecordingTransport::new(200, SUCCESS_BODY);
    let async_transport = RecordingTransport::new(200, SUCCESS_BODY);
    let sync_service = service(
        &settings,
        &context,
        &sync_transport,
        &NoTopaz,
        VariationPolicies::default(),
    );
    let async_service = service(
        &settings,
        &context,
        &async_transport,
        &NoTopaz,
        VariationPolicies::default(),
    );
    let mut call = variation_call();
    call.options = ignored_options();
    let sync_answer = image_variation(&sync_service, &call)
        .await
        .expect("the sync answer");
    let async_answer = aimage_variation(&async_service, &call)
        .await
        .expect("the async answer");
    assert_eq!(sync_answer, async_answer);
    let sync_requests = sync_transport.take();
    let async_requests = async_transport.take();
    let [sync_request] = sync_requests.as_slice() else {
        panic!("one sync request");
    };
    let [async_request] = async_requests.as_slice() else {
        panic!("one async request");
    };
    assert_eq!(sync_request.method, async_request.method);
    assert_eq!(sync_request.url, async_request.url);
    assert_eq!(sync_request.query, async_request.query);
    assert_eq!(sync_request.headers, async_request.headers);
    assert_eq!(sync_request.deadline, async_request.deadline);
    let sync_parts = multipart_parts(sync_request);
    let async_parts = multipart_parts(async_request);
    assert_eq!(sync_parts.len(), async_parts.len());
    for (left, right) in sync_parts.iter().zip(async_parts) {
        assert_eq!(
            (&left.name, &left.file_name, &left.content_type, &left.data),
            (
                &right.name,
                &right.file_name,
                &right.content_type,
                &right.data
            )
        );
    }
}

#[tokio::test]
async fn omitting_the_model_uses_dall_e_2() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("TOPAZ_API_KEY", "topaz-key")]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let topaz = RecordingTopaz::new(Some(response()));
    let service = service(
        &settings,
        &context,
        &transport,
        &topaz,
        VariationPolicies::default(),
    );
    let mut call = variation_call();
    call.custom_llm_provider = Some("topaz".to_owned());
    call.metadata = metadata();
    let answer = image_variation(&service, &call)
        .await
        .expect("the topaz variation succeeds");
    assert_eq!(answer, VariationAnswer::Response(response()));
    assert_eq!(transport.take().len(), 0, "the openai transport is unused");
    let requests = topaz.take();
    let [request] = requests.as_slice() else {
        panic!("one topaz request");
    };
    assert_eq!(request.model, "dall-e-2");
    assert_eq!(request.api_base, DEFAULT_TOPAZ_API_BASE);
    assert_eq!(request.api_key.expose_secret(), "topaz-key");
    assert!(request.optional_params.is_empty());
    assert_eq!(request.metadata, metadata());
    assert_eq!(request.timeout_seconds, 6000.0);
    assert!(!request.async_call);
}

#[tokio::test]
async fn a_declared_provider_prefixes_the_model() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("TOPAZ_API_KEY", "topaz-key")]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let topaz = RecordingTopaz::new(Some(response()));
    let service = service(
        &settings,
        &context,
        &transport,
        &topaz,
        VariationPolicies::default(),
    );
    let mut call = variation_call();
    call.custom_llm_provider = Some("topaz".to_owned());
    call.model = Some("dall-e-3".to_owned());
    let answer = image_variation(&service, &call)
        .await
        .expect("the declared provider wins over the inferred one");
    assert_eq!(answer, VariationAnswer::Response(response()));
    assert_eq!(transport.take().len(), 0, "the openai transport is unused");
    let requests = topaz.take();
    let [request] = requests.as_slice() else {
        panic!("one topaz request");
    };
    assert_eq!(request.model, "dall-e-3");
}

#[tokio::test]
async fn an_azure_chat_model_reaches_openai_despite_the_declaration() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("OPENAI_API_KEY", "sk-test")]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let topaz = RecordingTopaz::new(Some(response()));
    let service = service(
        &settings,
        &context,
        &transport,
        &topaz,
        VariationPolicies::default(),
    );
    let mut call = variation_call();
    call.custom_llm_provider = Some("topaz".to_owned());
    call.model = Some("azure/command-r".to_owned());
    let answer = image_variation(&service, &call)
        .await
        .expect("the azure chat branch resolves to openai");
    assert_eq!(answer, VariationAnswer::Response(response()));
    assert_eq!(topaz.take().len(), 0, "the topaz transport is unused");
    let requests = transport.take();
    let [request] = requests.as_slice() else {
        panic!("one openai request");
    };
    assert_eq!(
        request.url.as_str(),
        "https://api.openai.com/v1/images/variations"
    );
}

#[tokio::test]
async fn both_azure_conditions_are_required_to_skip_the_declared_prefix() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("TOPAZ_API_KEY", "topaz-key")]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let topaz = RecordingTopaz::new(Some(response()));
    let service = service(
        &settings,
        &context,
        &transport,
        &topaz,
        VariationPolicies::default(),
    );
    // an azure prefix over a model the catalog does not give to a chat provider is not the azure branch
    let mut prefix_only = variation_call();
    prefix_only.custom_llm_provider = Some("topaz".to_owned());
    prefix_only.model = Some("azure/mystery".to_owned());
    let answer = image_variation(&service, &prefix_only)
        .await
        .expect("the declared provider handles an azure prefix the catalog does not know");
    assert_eq!(answer, VariationAnswer::Response(response()));
    // a chat model under another prefix is not the azure branch either
    let mut chat_only = variation_call();
    chat_only.custom_llm_provider = Some("topaz".to_owned());
    chat_only.model = Some("legacy/command-r".to_owned());
    let answer = image_variation(&service, &chat_only)
        .await
        .expect("the declared provider handles a chat model outside the azure prefix");
    assert_eq!(answer, VariationAnswer::Response(response()));
    let requests = topaz.take();
    let models: Vec<&str> = requests
        .iter()
        .map(|request| request.model.as_str())
        .collect();
    assert_eq!(models, ["azure/mystery", "legacy/command-r"]);
    assert_eq!(transport.take().len(), 0);
}

#[tokio::test]
async fn an_empty_declared_provider_is_unset() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("OPENAI_API_KEY", "sk-test")]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let service = service(
        &settings,
        &context,
        &transport,
        &NoTopaz,
        VariationPolicies::default(),
    );
    let mut call = variation_call();
    call.custom_llm_provider = Some(String::new());
    let answer = image_variation(&service, &call)
        .await
        .expect("an empty declaration names no provider");
    assert_eq!(answer, VariationAnswer::Response(response()));
    assert_eq!(transport.take().len(), 1);
}

#[tokio::test]
async fn openai_receives_the_timeout_unchanged() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("OPENAI_API_KEY", "sk-test")]);
    for (timeout, expected) in [
        (None, Duration::from_secs(600)),
        (Some(0.0), Duration::ZERO),
        (Some(3.0), Duration::from_secs(3)),
    ] {
        let transport = RecordingTransport::new(200, SUCCESS_BODY);
        let service = service(
            &settings,
            &context,
            &transport,
            &NoTopaz,
            VariationPolicies::default(),
        );
        let mut call = variation_call();
        call.timeout = timeout;
        let answer = image_variation(&service, &call)
            .await
            .expect("the variation succeeds");
        assert!(matches!(answer, VariationAnswer::Response(_)));
        let requests = transport.take();
        let [request] = requests.as_slice() else {
            panic!("one request");
        };
        assert_eq!(request.deadline, expected);
    }
}

#[tokio::test]
async fn topaz_falls_back_to_the_default_request_timeout() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("TOPAZ_API_KEY", "topaz-key")]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let topaz = RecordingTopaz::new(Some(response()));
    let service = service(
        &settings,
        &context,
        &transport,
        &topaz,
        VariationPolicies::default(),
    );
    for timeout in [None, Some(0.0), Some(3.0)] {
        let mut call = variation_call();
        call.custom_llm_provider = Some("topaz".to_owned());
        call.timeout = timeout;
        let answer = image_variation(&service, &call)
            .await
            .expect("the topaz variation succeeds");
        assert!(matches!(answer, VariationAnswer::Response(_)));
    }
    let timeouts: Vec<f64> = topaz
        .take()
        .iter()
        .map(|request| request.timeout_seconds)
        .collect();
    assert_eq!(timeouts, [6000.0, 6000.0, 3.0]);
}

#[tokio::test]
async fn topaz_uses_the_configured_request_timeout() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(
        SOURCE,
        &[("TOPAZ_API_KEY", "topaz-key"), ("REQUEST_TIMEOUT", "7")],
    );
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let topaz = RecordingTopaz::new(Some(response()));
    let service = service(
        &settings,
        &context,
        &transport,
        &topaz,
        VariationPolicies::default(),
    );
    for timeout in [None, Some(0.0)] {
        let mut call = variation_call();
        call.custom_llm_provider = Some("topaz".to_owned());
        call.timeout = timeout;
        let answer = image_variation(&service, &call)
            .await
            .expect("the topaz variation succeeds");
        assert!(matches!(answer, VariationAnswer::Response(_)));
    }
    let timeouts: Vec<f64> = topaz
        .take()
        .iter()
        .map(|request| request.timeout_seconds)
        .collect();
    assert_eq!(timeouts, [7.0, 7.0]);
}

#[tokio::test]
async fn invalid_providers_raise_before_http() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("OPENAI_API_KEY", "sk-test")]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let service = service(
        &settings,
        &context,
        &transport,
        &NoTopaz,
        VariationPolicies::default(),
    );
    let mut azure = variation_call();
    azure.custom_llm_provider = Some("azure".to_owned());
    let error = image_variation(&service, &azure)
        .await
        .expect_err("azure is not a variation provider");
    assert_eq!(
        error.to_string(),
        "Invalid image variation provider: azure. Supported providers are: <enum 'LITELLM_IMAGE_VARIATION_PROVIDERS'>"
    );
    let mut anthropic = variation_call();
    anthropic.model = Some("anthropic/claude-2".to_owned());
    let error = image_variation(&service, &anthropic)
        .await
        .expect_err("anthropic is not a variation provider");
    assert_eq!(
        error.to_string(),
        "Invalid image variation provider: anthropic. Supported providers are: <enum 'LITELLM_IMAGE_VARIATION_PROVIDERS'>"
    );
    assert_eq!(transport.take().len(), 0);
}

#[tokio::test]
async fn missing_model_info_names_the_provider() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("OPENAI_API_KEY", "sk-test")]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let policies = VariationPolicies {
        model_info: |_, _| None,
        ..VariationPolicies::default()
    };
    let service = service(&settings, &context, &transport, &NoTopaz, policies);
    let error = image_variation(&service, &variation_call())
        .await
        .expect_err("openai has no model info config here");
    assert_eq!(
        error.to_string(),
        "image variation provider has no known model info config - required for getting api keys, etc.: openai. Supported providers are: <enum 'LITELLM_IMAGE_VARIATION_PROVIDERS'>"
    );
    assert_eq!(transport.take().len(), 0);
}

#[tokio::test]
async fn missing_keys_name_the_provider() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let service = service(
        &settings,
        &context,
        &transport,
        &NoTopaz,
        VariationPolicies::default(),
    );
    let error = image_variation(&service, &variation_call())
        .await
        .expect_err("openai has no key");
    assert_eq!(
        error.to_string(),
        "API key is required for OpenAI image variations"
    );
    let mut call = variation_call();
    call.custom_llm_provider = Some("topaz".to_owned());
    let error = image_variation(&service, &call)
        .await
        .expect_err("topaz has no key");
    assert_eq!(
        error.to_string(),
        "API key is required for Topaz image variations"
    );
    assert_eq!(transport.take().len(), 0);
}

#[tokio::test]
async fn null_bases_name_the_provider() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(
        SOURCE,
        &[
            ("OPENAI_API_KEY", "sk-test"),
            ("TOPAZ_API_KEY", "topaz-key"),
        ],
    );
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let policies = VariationPolicies {
        api_base: |_, _, _| None,
        ..VariationPolicies::default()
    };
    let service = service(&settings, &context, &transport, &NoTopaz, policies);
    let error = image_variation(&service, &variation_call())
        .await
        .expect_err("openai has no base");
    assert_eq!(
        error.to_string(),
        "API base is required for OpenAI image variations"
    );
    let mut call = variation_call();
    call.custom_llm_provider = Some("topaz".to_owned());
    let error = image_variation(&service, &call)
        .await
        .expect_err("topaz has no base");
    assert_eq!(
        error.to_string(),
        "API base is required for Topaz image variations"
    );
    assert_eq!(transport.take().len(), 0);
}

#[tokio::test]
async fn a_topaz_result_without_a_response_is_invalid() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("TOPAZ_API_KEY", "topaz-key")]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let topaz = RecordingTopaz::new(None);
    let service = service(
        &settings,
        &context,
        &transport,
        &topaz,
        VariationPolicies::default(),
    );
    let mut call = variation_call();
    call.custom_llm_provider = Some("topaz".to_owned());
    let error = image_variation(&service, &call)
        .await
        .expect_err("no response means no provider");
    assert_eq!(
        error.to_string(),
        "Invalid image variation provider: topaz. Supported providers are: <enum 'LITELLM_IMAGE_VARIATION_PROVIDERS'>"
    );
    assert_eq!(topaz.take().len(), 1);
}

#[tokio::test]
async fn negative_timeouts_are_rejected() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("OPENAI_API_KEY", "sk-test")]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let service = service(
        &settings,
        &context,
        &transport,
        &NoTopaz,
        VariationPolicies::default(),
    );
    let mut call = variation_call();
    call.timeout = Some(-1.0);
    let error = image_variation(&service, &call)
        .await
        .expect_err("a negative timeout is not a duration");
    assert_eq!(
        error.to_string(),
        "timeout must be a non-negative number of seconds, got -1"
    );
    assert_eq!(transport.take().len(), 0);
}

#[tokio::test]
async fn bad_request_variations_project_a_bad_request() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("OPENAI_API_KEY", "sk-test")]);
    let transport = RecordingTransport::new(400, ERROR_BODY);
    let service = service(
        &settings,
        &context,
        &transport,
        &NoTopaz,
        VariationPolicies::default(),
    );
    let error = aimage_variation(&service, &variation_call())
        .await
        .expect_err("the provider refuses the request");
    let metadata = error.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
    assert!(matches!(error.class(), ErrorClass::BadRequest(_)));
    assert!(error.to_string().contains("nope"));
    assert_eq!(transport.take().len(), 1);
}

#[tokio::test]
async fn rate_limited_variations_project_without_a_retry() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("OPENAI_API_KEY", "sk-test")]);
    let transport = RecordingTransport::new(429, ERROR_BODY);
    let service = service(
        &settings,
        &context,
        &transport,
        &NoTopaz,
        VariationPolicies::default(),
    );
    let error = aimage_variation(&service, &variation_call())
        .await
        .expect_err("the provider rate limits the call");
    let metadata = error.metadata();
    assert_eq!(metadata.status_code, 429);
    assert_eq!(
        metadata.error_type,
        WireField::Text("throttling_error".to_owned())
    );
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code.as_deref(), Some("429"));
    assert!(matches!(error.class(), ErrorClass::RateLimit(_)));
    assert_eq!(transport.take().len(), 1, "the wrapper does not retry");
}

#[tokio::test]
async fn sync_variation_errors_keep_the_provider_status() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("OPENAI_API_KEY", "sk-test")]);
    let transport = RecordingTransport::new(400, ERROR_BODY);
    let service = service(
        &settings,
        &context,
        &transport,
        &NoTopaz,
        VariationPolicies::default(),
    );
    let error = image_variation(&service, &variation_call())
        .await
        .expect_err("the provider refuses the request");
    let ImageVariationError::OpenAi(source) = error else {
        panic!("a provider error keeps its status");
    };
    assert_eq!(source.status_code, 400);
}

#[tokio::test]
async fn async_call_forces_the_flag_and_keeps_the_context() {
    let operation = ScriptedOperation::new(vec![Scripted::Response(response())]);
    let mut call = variation_call();
    call.metadata = metadata();
    let answer = aimage_variation(&operation, &call)
        .await
        .expect("the scripted answer");
    assert_eq!(answer, VariationAnswer::Response(response()));
    assert!(!call.async_call, "the caller's call is not changed");
    let calls = recorded_calls(&operation);
    let [recorded] = calls.as_slice() else {
        panic!("one sync call");
    };
    assert!(recorded.async_call);
    assert_eq!(recorded.metadata, metadata());
}

#[tokio::test]
async fn dict_answers_normalize_once() {
    let mapping: Json =
        serde_json::from_str(r#"{"created":1,"data":[{"url":"https://example.test/a.png"}]}"#)
            .expect("the cached answer parses");
    let operation = ScriptedOperation::new(vec![Scripted::Mapping(mapping)]);
    let answer = aimage_variation(&operation, &variation_call())
        .await
        .expect("the dict normalizes");
    let expected = ImageResponse {
        created: 1,
        data: Some(vec![ImageData {
            url: Some("https://example.test/a.png".to_owned()),
            ..ImageData::default()
        }]),
        usage: None,
    };
    assert_eq!(answer, VariationAnswer::Response(expected));
    assert_eq!(recorded_calls(&operation).len(), 1);
    assert_eq!(operation.awaits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn pending_answers_are_awaited_once() {
    let operation = ScriptedOperation::new(vec![Scripted::Pending(response())]);
    let answer = aimage_variation(&operation, &variation_call())
        .await
        .expect("the pending answer resolves");
    assert_eq!(answer, VariationAnswer::Response(response()));
    assert_eq!(recorded_calls(&operation).len(), 1);
    assert_eq!(operation.awaits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn response_answers_pass_through() {
    let operation = ScriptedOperation::new(vec![Scripted::Response(response())]);
    let answer = aimage_variation(&operation, &variation_call())
        .await
        .expect("the response passes through");
    assert_eq!(answer, VariationAnswer::Response(response()));
    assert_eq!(recorded_calls(&operation).len(), 1);
    assert_eq!(operation.awaits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unexpected_answers_repeat_the_sync_call() {
    let operation = ScriptedOperation::new(vec![
        Scripted::Unexpected(Json::Number(Number::from(5))),
        Scripted::Unexpected(Json::Number(Number::from(7))),
    ]);
    let answer = aimage_variation(&operation, &variation_call())
        .await
        .expect("the second answer is returned unchecked");
    assert_eq!(
        answer,
        VariationAnswer::Unexpected(Json::Number(Number::from(7)))
    );
    let calls = recorded_calls(&operation);
    assert_eq!(calls.len(), 2, "the sync call runs twice");
    assert!(calls.iter().all(|call| call.async_call));
    assert_eq!(operation.awaits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn dict_answers_with_wrong_fields_raise_the_wrapper_exception() {
    let mapping: Json =
        serde_json::from_str(r#"{"created":"soon"}"#).expect("the broken answer parses");
    let operation = ScriptedOperation::new(vec![Scripted::Mapping(mapping)]);
    let error = aimage_variation(&operation, &variation_call())
        .await
        .expect_err("the fields do not normalize");
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn async_failures_default_to_openai() {
    let operation = FailingOperation::new(None);
    let error = aimage_variation(&operation, &variation_call())
        .await
        .expect_err("the operation fails");
    assert_eq!(error.metadata().provider.as_deref(), Some("openai"));
    assert_eq!(
        operation.inferences.load(Ordering::SeqCst),
        0,
        "no inference without a model"
    );
}

#[tokio::test]
async fn async_failures_infer_a_provider_only_with_a_model() {
    let operation = FailingOperation::new(Some("topaz"));
    let mut call = variation_call();
    call.model = Some("dall-e-2".to_owned());
    let error = aimage_variation(&operation, &call)
        .await
        .expect_err("the operation fails");
    assert_eq!(error.metadata().provider.as_deref(), Some("topaz"));
    assert_eq!(operation.inferences.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn async_failures_prefer_the_declared_provider() {
    let operation = FailingOperation::new(Some("topaz"));
    let mut call = variation_call();
    call.model = Some("dall-e-2".to_owned());
    call.custom_llm_provider = Some("azure".to_owned());
    let error = aimage_variation(&operation, &call)
        .await
        .expect_err("the operation fails");
    assert_eq!(error.metadata().provider.as_deref(), Some("azure"));
    assert_eq!(
        operation.inferences.load(Ordering::SeqCst),
        0,
        "a declared provider skips the inference"
    );
}

#[tokio::test]
async fn async_failures_use_the_provider_the_model_resolves_to() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let service = service(
        &settings,
        &context,
        &transport,
        &RecordingTopaz::new(Some(response())),
        VariationPolicies::default(),
    );
    let mut call = variation_call();
    call.model = Some("topaz/dall-e-3".to_owned());
    let error = aimage_variation(&service, &call)
        .await
        .expect_err("the topaz key is missing");
    assert_eq!(error.metadata().provider.as_deref(), Some("topaz"));
    assert_eq!(transport.take().len(), 0);
}

#[test]
fn openai_key_falls_through_its_sources() {
    let policies = VariationPolicies::default();
    let all = settings(
        "litellm_settings:\n  api_key: yaml-key\n  openai_key: yaml-openai-key\n",
        &[("OPENAI_API_KEY", "env-key")],
    );
    let supplied = SecretString::from("supplied");
    let key = (policies.api_key)(ProviderId::Openai, Some(&supplied), &all).expect("supplied key");
    assert_eq!(key.expose_secret(), "supplied");
    let key = (policies.api_key)(ProviderId::Openai, None, &all).expect("global key");
    assert_eq!(key.expose_secret(), "yaml-key");
    let openai_key_only = settings(
        "litellm_settings:\n  openai_key: yaml-openai-key\n",
        &[("OPENAI_API_KEY", "env-key")],
    );
    let key = (policies.api_key)(ProviderId::Openai, None, &openai_key_only).expect("openai_key");
    assert_eq!(key.expose_secret(), "yaml-openai-key");
    let environment_only = settings(SOURCE, &[("OPENAI_API_KEY", "env-key")]);
    let key =
        (policies.api_key)(ProviderId::Openai, None, &environment_only).expect("environment key");
    assert_eq!(key.expose_secret(), "env-key");
    let empty = SecretString::from("");
    let key = (policies.api_key)(ProviderId::Openai, Some(&empty), &environment_only)
        .expect("an empty key falls through");
    assert_eq!(key.expose_secret(), "env-key");
    let none = settings(SOURCE, &[]);
    assert!((policies.api_key)(ProviderId::Openai, None, &none).is_none());
}

#[test]
fn openai_base_falls_through_its_sources() {
    let policies = VariationPolicies::default();
    let all = settings(
        "litellm_settings:\n  api_base: https://yaml.example/v1\n",
        &[
            ("OPENAI_BASE_URL", "https://env.example/v1"),
            ("OPENAI_API_BASE", "https://legacy.example/v1"),
        ],
    );
    let supplied = (policies.api_base)(ProviderId::Openai, Some("https://call.example/v1"), &all);
    assert_eq!(supplied.as_deref(), Some("https://call.example/v1"));
    assert_eq!(
        (policies.api_base)(ProviderId::Openai, None, &all).as_deref(),
        Some("https://yaml.example/v1")
    );
    let environment_only = settings(
        SOURCE,
        &[
            ("OPENAI_BASE_URL", "https://env.example/v1"),
            ("OPENAI_API_BASE", "https://legacy.example/v1"),
        ],
    );
    assert_eq!(
        (policies.api_base)(ProviderId::Openai, None, &environment_only).as_deref(),
        Some("https://env.example/v1")
    );
    let legacy_only = settings(SOURCE, &[("OPENAI_API_BASE", "https://legacy.example/v1")]);
    assert_eq!(
        (policies.api_base)(ProviderId::Openai, None, &legacy_only).as_deref(),
        Some("https://legacy.example/v1")
    );
    assert_eq!(
        (policies.api_base)(ProviderId::Openai, None, &settings(SOURCE, &[])).as_deref(),
        Some(DEFAULT_OPENAI_API_BASE)
    );
    assert_eq!(
        (policies.api_base)(ProviderId::Openai, Some(""), &all).as_deref(),
        Some("https://yaml.example/v1")
    );
}

#[test]
fn topaz_credentials_fall_through_their_sources() {
    let policies = VariationPolicies::default();
    let settings = settings(SOURCE, &[("TOPAZ_API_KEY", "topaz-env-key")]);
    let key = (policies.api_key)(ProviderId::Topaz, None, &settings).expect("topaz key");
    assert_eq!(key.expose_secret(), "topaz-env-key");
    assert_eq!(
        (policies.api_base)(ProviderId::Topaz, None, &settings).as_deref(),
        Some(DEFAULT_TOPAZ_API_BASE)
    );
    let supplied = SecretString::from("supplied-topaz");
    let key = (policies.api_key)(ProviderId::Topaz, Some(&supplied), &settings)
        .expect("supplied topaz key");
    assert_eq!(key.expose_secret(), "supplied-topaz");
    let supplied_base =
        (policies.api_base)(ProviderId::Topaz, Some("https://topaz.example"), &settings);
    assert_eq!(supplied_base.as_deref(), Some("https://topaz.example"));
    let with_base = settings(SOURCE, &[("TOPAZ_API_BASE", "https://topaz.example")]);
    assert_eq!(
        (policies.api_base)(ProviderId::Topaz, None, &with_base).as_deref(),
        Some("https://topaz.example")
    );
    let none = settings(SOURCE, &[]);
    assert!((policies.api_key)(ProviderId::Topaz, None, &none).is_none());
    assert!((policies.api_key)(ProviderId::Azure, None, &settings).is_none());
    assert!((policies.api_base)(ProviderId::Azure, None, &settings).is_none());
}
