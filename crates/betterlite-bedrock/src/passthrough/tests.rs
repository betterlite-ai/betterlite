#![cfg(test)]

use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use betterlite_context::chat_config_selection::selection::ChatConfigError;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, PassthroughSigning, PassthroughTarget, PassthroughUrlRequest,
    RelayCallType, RelayedResponse,
};
use betterlite_streaming::stream_base_assembly::provider_metadata::ChatConfigSelection;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use rstest::rstest;
use secrecy::SecretString;
use uuid::Uuid;

use crate::aws_settings::sources::Credentials;

use super::collector::{
    DECODER_FLAGS, DecoderChoice, DecoderFlags, StreamLogAssembler, decoder_box, decoder_selection,
};
use super::model_id::{encoded_model_id, with_model_id};
use super::record::{DecodedChunk, EventChunkDecoder};
use super::reply::{NativeReplyRequest, NativeReplyTransform, logged_reply};
use super::url::PassthroughEnvironment;
use super::{BedrockPassthroughPlan, CollectorClock, PassthroughIdentity};

mod collecting;
mod runs;
mod safety;

use collecting::event_stream_collector;
use runs::{plain_text_delta_rules, text_run_sealing};

const HOST: &str = "https://bedrock-runtime.us-east-1.amazonaws.com";

fn fixture_time() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .unwrap()
}

fn fixture_id() -> Uuid {
    Uuid::from_u128(7)
}

fn fixture_now() -> i64 {
    1_700_000_000
}

fn fixture_instant() -> StreamInstant {
    StreamInstant {
        seconds: 1_700_000_000,
        timestamp: 1_700_000_000.0,
    }
}

fn clock() -> CollectorClock {
    CollectorClock {
        next_id: fixture_id,
        now: fixture_now,
        instant: fixture_instant,
    }
}

struct TestLogging;

impl PassthroughLogging for TestLogging {
    fn set_call_type(&mut self, _call_type: RelayCallType) {}
}

struct TestSelector {
    tag: Option<ChatConfigTag>,
    calls: Mutex<Vec<(String, ProviderId)>>,
}

impl ChatConfigSelection for TestSelector {
    fn select_chat_config(
        &self,
        model: &str,
        provider: ProviderId,
    ) -> Result<Option<ChatConfigTag>, ChatConfigError> {
        self.calls
            .lock()
            .unwrap()
            .push((model.to_owned(), provider));
        Ok(self.tag)
    }
}

struct RecordedReply {
    tag: ChatConfigTag,
    model: String,
    provider: String,
    status: u16,
    body: Vec<u8>,
    messages: Vec<(String, String)>,
    request_data: Object,
    optional_empty: bool,
    litellm_empty: bool,
    api_key: String,
}

struct TestTransform {
    recorded: Mutex<Vec<RecordedReply>>,
    result: Json,
    fail: bool,
}

impl NativeReplyTransform for TestTransform {
    fn transform(&self, request: &NativeReplyRequest<'_>) -> Result<Json, Box<ProviderError>> {
        self.recorded.lock().unwrap().push(RecordedReply {
            tag: request.tag,
            model: request.model.to_owned(),
            provider: request.custom_llm_provider.to_owned(),
            status: request.status_code,
            body: request.body.to_vec(),
            messages: request
                .messages
                .iter()
                .map(|message| (message.role.to_owned(), message.content.to_owned()))
                .collect(),
            request_data: request.request_data.clone(),
            optional_empty: request.optional_params.is_empty(),
            litellm_empty: request.litellm_params.is_empty(),
            api_key: request.api_key.to_owned(),
        });
        if self.fail {
            return Err(Box::new(ProviderError::from(ProviderErrorParts {
                status_code: 400,
                message: "the transform failed with FIXTURE-SECRET".to_owned(),
                headers: None,
                request: None,
                response: None,
                body: None,
                status_code_is_synthesized: false,
            })));
        }
        Ok(self.result.clone())
    }
}

struct TestAssembler {
    chunks: Mutex<Vec<StreamChunk>>,
    calls: Mutex<usize>,
    marker: Json,
}

impl StreamLogAssembler for TestAssembler {
    fn assemble(
        &self,
        chunks: &[StreamChunk],
        _logging_obj: &mut dyn PassthroughLogging,
    ) -> Option<Json> {
        *self.chunks.lock().unwrap() = chunks.to_vec();
        let mut calls = self.calls.lock().unwrap();
        *calls = calls.saturating_add(1);
        Some(self.marker.clone())
    }
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn credentials(access: &str, secret: &str) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(access.to_owned()),
        secret_access_key: SecretString::from(secret.to_owned()),
        session_token: None,
    }
}

fn identity() -> PassthroughIdentity {
    PassthroughIdentity {
        credentials: Some(credentials("AKIAFIXTURE", "fixture-secret")),
        bearer_token: None,
        now: fixture_time(),
    }
}

fn environment() -> PassthroughEnvironment {
    PassthroughEnvironment {
        region_name: Some("us-east-1".to_owned()),
        region: None,
        session_region: None,
        runtime_endpoint: None,
    }
}

fn target<'text>(model: &'text str, endpoint: &'text str) -> PassthroughTarget<'text> {
    PassthroughTarget {
        model,
        custom_llm_provider: "bedrock",
        endpoint,
    }
}

fn plan<'services>(
    selector: &'services TestSelector,
    transform: &'services TestTransform,
    assembler: &'services TestAssembler,
    environment: PassthroughEnvironment,
    identity: PassthroughIdentity,
) -> BedrockPassthroughPlan<'services> {
    BedrockPassthroughPlan {
        environment,
        identity,
        selector,
        transform,
        assembler,
        clock: clock(),
    }
}

fn fixtures() -> (TestSelector, TestTransform, TestAssembler) {
    (
        TestSelector {
            tag: Some(ChatConfigTag::Anthropic),
            calls: Mutex::new(Vec::new()),
        },
        TestTransform {
            recorded: Mutex::new(Vec::new()),
            result: json(r#"{"id":"logged"}"#),
            fail: false,
        },
        TestAssembler {
            chunks: Mutex::new(Vec::new()),
            calls: Mutex::new(0),
            marker: json(r#"{"id":"assembled"}"#),
        },
    )
}

fn url_request<'text>(
    target: PassthroughTarget<'text>,
    litellm_params: &'text Object,
    api_base: Option<&'text str>,
) -> PassthroughUrlRequest<'text> {
    PassthroughUrlRequest {
        api_base,
        api_key: None,
        target,
        request_query_params: None,
        litellm_params,
    }
}

fn url_of(config: &BedrockPassthroughPlan<'_>, request: &PassthroughUrlRequest<'_>) -> String {
    config.get_complete_url(request).unwrap().0.to_string()
}

fn url_for(config: &BedrockPassthroughPlan<'_>, endpoint: &str, params: &str) -> String {
    let litellm_params = object(params);
    let request = url_request(
        target("anthropic.claude-3", endpoint),
        &litellm_params,
        None,
    );
    url_of(config, &request)
}

fn url_with(
    config: &BedrockPassthroughPlan<'_>,
    model: &str,
    params: &str,
    api_base: Option<&str>,
) -> String {
    let litellm_params = object(params);
    let request = url_request(target(model, "model/x/converse"), &litellm_params, api_base);
    url_of(config, &request)
}

fn signing_context<'text>(
    litellm_params: &'text Object,
    request_data: Option<&'text Object>,
) -> PassthroughSigning<'text> {
    PassthroughSigning {
        litellm_params,
        request_data,
        api_base: "https://bedrock-runtime.us-east-1.amazonaws.com/model/m/converse",
        model: Some("m"),
    }
}

fn default_plan<'services>(
    services: &'services (TestSelector, TestTransform, TestAssembler),
) -> BedrockPassthroughPlan<'services> {
    plan(
        &services.0,
        &services.1,
        &services.2,
        environment(),
        identity(),
    )
}

fn invoke_id_encoding() {
    let services = fixtures();
    let config = default_plan(&services);
    let arn =
        r#"{"model_id":"arn:aws:bedrock:us-east-1:123:application-inference-profile/abc123"}"#;
    assert_eq!(
        url_for(&config, "model/anthropic.claude-3/invoke", arn),
        format!(
            "{HOST}/model/arn:aws:bedrock:us-east-1:123:application-inference-profile%2Fabc123/invoke"
        )
    );
    let slashed = r#"{"model_id":"custom/id"}"#;
    assert_eq!(
        url_for(&config, "model/anthropic.claude-3/invoke", slashed),
        format!("{HOST}/model/custom/invoke")
    );
    assert_eq!(encoded_model_id(""), "");
    assert_eq!(encoded_model_id("/model/a"), "a");
    assert_eq!(
        encoded_model_id("arn:aws:bedrock:eu-west-1:1:foundation-model/x"),
        "arn:aws:bedrock:eu-west-1:1:foundation-model%2Fx"
    );
}

fn converse_id_encoding() {
    let services = fixtures();
    let config = default_plan(&services);
    let endpoint = "model/anthropic.claude-3/converse";
    let cases = [
        ("{}", endpoint, "model/anthropic.claude-3/converse"),
        (
            r#"{"model_id":null}"#,
            endpoint,
            "model/anthropic.claude-3/converse",
        ),
        (r#"{"model_id":""}"#, endpoint, "model//converse"),
        (
            r#"{"model_id":"abc%2Fdef"}"#,
            endpoint,
            "model/abc%2Fdef/converse",
        ),
        (
            r#"{"model_id":"replacement"}"#,
            "custom/converse",
            "custom/converse",
        ),
    ];
    for (params, source, path) in cases {
        assert_eq!(
            url_for(&config, source, params),
            format!("{HOST}/{path}"),
            "{params}"
        );
    }
    assert_eq!(with_model_id("model/a/model/b/x", "z"), "model/z/model/z/x");
    assert_eq!(with_model_id("model//x", "z"), "model//x");
    assert_eq!(with_model_id("model///x", "z"), "model///x");
    let wrong = object(r#"{"model_id":5}"#);
    let request = url_request(
        target("anthropic.claude-3", "model/x/converse"),
        &wrong,
        None,
    );
    let failure = config.get_complete_url(&request).unwrap_err();
    assert_eq!(failure.to_string(), "the field model_id is not a string");
}

fn signed_with_the_configured_bearer() {
    let services = fixtures();
    let mut configured = identity();
    configured.bearer_token = Some(SecretString::from("fixture-bearer".to_owned()));
    let config = plan(
        &services.0,
        &services.1,
        &services.2,
        environment(),
        configured,
    );
    let params = object(r#"{"api_key":"sk-should-not-sign"}"#);
    let data = object(r#"{"input":"hi"}"#);
    let mut headers = Headers::new();
    headers.insert("x-custom".to_owned(), "kept".to_owned());
    let context = signing_context(&params, Some(&data));
    let (signed, body) = config.sign_request(headers, &context).unwrap();
    assert_eq!(signed["Authorization"], "Bearer fixture-bearer");
    assert_eq!(signed["Content-Type"], "application/json");
    assert_eq!(signed["x-custom"], "kept");
    assert!(
        !signed
            .keys()
            .any(|name| name.eq_ignore_ascii_case("content-length"))
    );
    assert_eq!(
        String::from_utf8(body.unwrap()).unwrap(),
        r#"{"input": "hi"}"#
    );
}

fn signed_with_sigv4_credentials() {
    let services = fixtures();
    let config = default_plan(&services);
    let params = object(r#"{"aws_region_name":"eu-west-1"}"#);
    let (signed, body) = config
        .sign_request(Headers::new(), &signing_context(&params, None))
        .unwrap();
    let authorization = &signed["Authorization"];
    assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIAFIXTURE/"));
    assert!(authorization.contains("/eu-west-1/bedrock/aws4_request"));
    assert_eq!(signed["Content-Type"], "application/json");
    assert!(
        !signed
            .keys()
            .any(|name| name.eq_ignore_ascii_case("content-length"))
    );
    assert_eq!(String::from_utf8(body.unwrap()).unwrap(), "{}");
}

fn supplied_content_type_is_kept() {
    let services = fixtures();
    let config = default_plan(&services);
    let params = object(r#"{"aws_region_name":"eu-west-1"}"#);
    let mut supplied = Headers::new();
    supplied.insert("Content-Type".to_owned(), "text/plain".to_owned());
    let (kept, _) = config
        .sign_request(supplied, &signing_context(&params, None))
        .unwrap();
    assert_eq!(kept["Content-Type"], "text/plain");
    let types = kept
        .keys()
        .filter(|name| name.eq_ignore_ascii_case("content-type"));
    assert_eq!(types.count(), 1);
}

fn caller_authorization_and_length_win() {
    let services = fixtures();
    let config = default_plan(&services);
    let params = object(r#"{"aws_region_name":"eu-west-1"}"#);
    let mut supplied = Headers::new();
    supplied.insert("authorization".to_owned(), "Bearer supplied".to_owned());
    supplied.insert("Content-Length".to_owned(), "5".to_owned());
    let context = signing_context(&params, None);
    let (signed, _) = config.sign_request(supplied, &context).unwrap();
    assert_eq!(signed["Authorization"], "Bearer supplied");
    assert!(!signed.contains_key("authorization"));
    assert_eq!(signed["Content-Length"], "5");
}

fn error_class_keeps_status_message_and_headers() {
    let services = fixtures();
    let config = default_plan(&services);
    let mut headers = Headers::new();
    headers.insert("x-amzn-requestid".to_owned(), "abc".to_owned());
    let error = config.get_error_class("denied", 403, &headers);
    assert_eq!(error.status_code, 403);
    assert_eq!(error.message, "denied");
    assert!(!error.status_code_is_synthesized);
    assert_eq!(
        error.headers,
        Some(vec![("x-amzn-requestid".to_owned(), "abc".to_owned())])
    );
    assert_eq!(
        config
            .get_error_class("denied", 403, &Headers::new())
            .headers,
        None
    );
}

fn bearer_selection() {
    signed_with_the_configured_bearer();
    signed_with_sigv4_credentials();
    supplied_content_type_is_kept();
    caller_authorization_and_length_win();
    error_class_keeps_status_message_and_headers();
}

fn log_with(config: &BedrockPassthroughPlan<'_>, endpoint: &str) -> Option<Json> {
    let response = RelayedResponse {
        status_code: 200,
        body: b"{\"id\":\"native\"}",
    };
    let request_data = object(r#"{"input":"hi"}"#);
    let call_target = target("anthropic.claude-3", endpoint);
    config.logging_non_streaming_response(&call_target, &response, &request_data, &mut TestLogging)
}

fn invoke_endpoint_logs_through_its_config() {
    let services = fixtures();
    let config = default_plan(&services);
    let logged = log_with(&config, "model/anthropic.claude-3/invoke");
    assert_eq!(logged, Some(json(r#"{"id":"logged"}"#)));
    let first = services.1.recorded.lock().unwrap().remove(0);
    assert_eq!(first.tag, ChatConfigTag::Anthropic);
    assert_eq!(first.model, "anthropic.claude-3");
    assert_eq!(first.provider, "bedrock");
    assert_eq!(first.status, 200);
    assert_eq!(first.body, b"{\"id\":\"native\"}");
    let message = (
        "user".to_owned(),
        "no-message-pass-through-endpoint".to_owned(),
    );
    assert_eq!(first.messages, vec![message]);
    assert!(first.optional_empty);
    assert!(first.litellm_empty);
    assert_eq!(first.api_key, "");
    assert_eq!(first.request_data, object(r#"{"input":"hi"}"#));
    let (model, provider) = services.0.calls.lock().unwrap().remove(0);
    assert_eq!(model, "invoke/anthropic.claude-3");
    assert_eq!(provider, ProviderId::Bedrock);
}

fn endpoint_decides_the_chat_config_model() {
    let services = fixtures();
    let config = default_plan(&services);
    for endpoint in [
        "model/x/converse",
        "model/x/invoke/converse",
        "model/x/invoke",
    ] {
        assert!(log_with(&config, endpoint).is_some(), "{endpoint}");
    }
    let calls = services.0.calls.lock().unwrap();
    let models: Vec<&str> = calls.iter().map(|(model, _)| model.as_str()).collect();
    assert_eq!(
        models,
        [
            "converse/anthropic.claude-3",
            "invoke/anthropic.claude-3",
            "invoke/anthropic.claude-3"
        ]
    );
}

fn other_endpoints_are_not_logged() {
    let services = fixtures();
    let config = default_plan(&services);
    assert!(log_with(&config, "model/x").is_none());
    assert!(services.0.calls.lock().unwrap().is_empty());
    assert!(services.1.recorded.lock().unwrap().is_empty());
}

fn selection_failures_are_reported() {
    let services = fixtures();
    let no_config = TestSelector {
        tag: None,
        calls: Mutex::new(Vec::new()),
    };
    let missing = plan(
        &no_config,
        &services.1,
        &services.2,
        environment(),
        identity(),
    );
    let response = RelayedResponse {
        status_code: 200,
        body: b"{}",
    };
    let request_data = Object::new();
    let invoke = target("anthropic.claude-3", "model/x/invoke");
    let failure = logged_reply(&missing, &invoke, &response, &request_data).unwrap_err();
    assert_eq!(
        failure.to_string(),
        "No provider config found for model: anthropic.claude-3"
    );
    let odd = PassthroughTarget {
        model: "m",
        custom_llm_provider: "not-a-provider",
        endpoint: "model/m/invoke",
    };
    let unknown = logged_reply(&missing, &odd, &response, &request_data).unwrap_err();
    assert_eq!(
        unknown.to_string(),
        "the provider not-a-provider is not a known provider"
    );
}

fn transform_failure_is_logged_not_raised() {
    let services = fixtures();
    let mut failing = fixtures().1;
    failing.fail = true;
    let config = plan(
        &services.0,
        &failing,
        &services.2,
        environment(),
        identity(),
    );
    assert!(log_with(&config, "model/x/invoke").is_none());
}

fn native_reply_log() {
    invoke_endpoint_logs_through_its_config();
    endpoint_decides_the_chat_config_model();
    other_endpoints_are_not_logged();
    selection_failures_are_reported();
    transform_failure_is_logged_not_raised();
}

fn endpoint_sources() {
    let services = fixtures();
    let config = default_plan(&services);
    let proxy = Some("https://proxy.example/base/");
    let expected = "https://proxy.example/base/model/x/converse";
    assert_eq!(
        url_with(&config, "anthropic.claude-3", "{}", proxy),
        expected
    );
    let configured = r#"{"aws_bedrock_runtime_endpoint":"https://configured.example"}"#;
    let expected = "https://configured.example/model/x/converse";
    assert_eq!(
        url_with(&config, "anthropic.claude-3", configured, None),
        expected
    );
    let mut environed = environment();
    environed.runtime_endpoint = Some("https://env.example".to_owned());
    let env_config = plan(&services.0, &services.1, &services.2, environed, identity());
    let expected = "https://env.example/model/x/converse";
    assert_eq!(
        url_with(&env_config, "anthropic.claude-3", "{}", None),
        expected
    );
    let query = object(r#"{"beta":"1"}"#);
    let params = object("{}");
    let request = PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: target("anthropic.claude-3", "model/x/converse"),
        request_query_params: Some(&query),
        litellm_params: &params,
    };
    assert_eq!(
        url_of(&env_config, &request),
        "https://env.example/model/x/converse?beta=1"
    );
}

fn region_sources() {
    let services = fixtures();
    let config = plan(
        &services.0,
        &services.1,
        &services.2,
        PassthroughEnvironment::default(),
        identity(),
    );
    let arn = "arn:aws:bedrock:eu-west-1:1:foundation-model/anthropic.claude";
    let expected = "https://bedrock-runtime.eu-west-1.amazonaws.com/model/x/converse";
    assert_eq!(url_with(&config, arn, "{}", None), expected);
    let explicit = r#"{"aws_region_name":"ap-south-1"}"#;
    let expected = "https://bedrock-runtime.ap-south-1.amazonaws.com/model/x/converse";
    assert_eq!(url_with(&config, arn, explicit, None), expected);
    let profile = r#"{"model_id":"arn:aws:bedrock:ap-south-1:1:application-inference-profile/x"}"#;
    let expected = "https://bedrock-runtime.ap-south-1.amazonaws.com/model/arn:aws:bedrock:ap-south-1:1:application-inference-profile%2Fx/converse";
    assert_eq!(
        url_with(&config, "anthropic.claude-3", profile, None),
        expected
    );
    let wrong = object(r#"{"aws_region_name":5}"#);
    let request = url_request(
        target("anthropic.claude-3", "model/x/converse"),
        &wrong,
        None,
    );
    let failure = config.get_complete_url(&request).unwrap_err();
    assert_eq!(
        failure.to_string(),
        "the field aws_region_name is not a string"
    );
}

fn streaming_detection_is_a_substring_check() {
    let services = fixtures();
    let config = default_plan(&services);
    let body = object(r#"{"stream":true}"#);
    assert!(config.is_streaming_request("model/x/stream", &Object::new()));
    assert!(config.is_streaming_request("model/x/converse-stream", &Object::new()));
    assert!(!config.is_streaming_request("model/x/invoke", &body));
    assert!(!config.is_streaming_request("model/x/STREAM", &Object::new()));
}

fn passthrough_url() {
    endpoint_sources();
    region_sources();
    streaming_detection_is_a_substring_check();
}

fn decode_chunk(decoder: &mut dyn EventChunkDecoder, source: &str) -> Box<StreamChunk> {
    match decoder.decode(&json(source), clock()).unwrap() {
        DecodedChunk::Chunk(chunk) => chunk,
        DecodedChunk::Generic(_) | DecodedChunk::Ignored => panic!("expected a translated chunk"),
    }
}

fn invoke_provider_selection() {
    let invoke = decoder_selection("anthropic.claude-3", "model/x/invoke", clock());
    let DecoderChoice::Invoke {
        provider,
        mut decoder,
    } = invoke
    else {
        panic!("expected an invoke decoder");
    };
    assert_eq!(provider.name(), "anthropic");
    let event =
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#;
    let chunk = decode_chunk(&mut decoder, event);
    assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hi"));
    assert_eq!(chunk.model, "");
    let missing = decoder_selection("mystery.v9", "model/x/invoke", clock());
    let DecoderChoice::Disabled { warning } = missing else {
        panic!("expected a disabled decoder");
    };
    let message =
        "Bedrock passthrough spend tracking skipped: no invoke provider for model mystery.v9";
    assert_eq!(warning.as_deref(), Some(message));
    let both = decoder_selection("anthropic.claude-3", "model/x/invoke/converse", clock());
    assert!(matches!(both, DecoderChoice::Invoke { .. }));
}

fn converse_and_other_endpoints() {
    let converse = decoder_selection("mystery.v9", "model/x/converse", clock());
    let DecoderChoice::Converse(mut decoder) = converse else {
        panic!("expected a converse decoder");
    };
    let text = r#"{"contentBlockIndex":0,"delta":{"text":"hi"}}"#;
    let chunk = decode_chunk(&mut decoder, text);
    assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hi"));
    assert_eq!(chunk.model, "mystery.v9");
    assert!(chunk.provider_specific_fields.is_none());
    let traced = r#"{"contentBlockIndex":0,"delta":{"text":"hi"},"trace":{"guardrail":{"a":1}}}"#;
    let with_trace = decode_chunk(&mut decoder, traced);
    let fields = with_trace.provider_specific_fields.unwrap();
    assert!(fields.contains_key("trace"));
    let other = decoder_selection("mystery.v9", "model/x/other", clock());
    let DecoderChoice::Disabled { warning } = other else {
        panic!("expected a disabled decoder");
    };
    assert_eq!(warning, None);
    assert!(decoder_box(DecoderChoice::Disabled { warning: None }).is_none());
}

fn legacy_factory_has_no_moonshot_branch() {
    let moonshot = decoder_selection("moonshot.v1", "model/moonshot.v1/invoke", clock());
    let DecoderChoice::Invoke {
        provider,
        mut decoder,
    } = moonshot
    else {
        panic!("expected an invoke decoder");
    };
    assert_eq!(provider.name(), "moonshot");
    let text = r#"{"contentBlockIndex":0,"delta":{"text":"hi"}}"#;
    let chunk = decode_chunk(&mut decoder, text);
    assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hi"));
    let boxed = decoder_box(decoder_selection(
        "anthropic.claude-3",
        "model/x/converse",
        clock(),
    ));
    let stop = decode_chunk(boxed.unwrap().as_mut(), r#"{"stopReason":"end_turn"}"#);
    assert_eq!(stop.choices[0].finish_reason.as_deref(), Some("stop"));
}

fn collector_decoder_selection() {
    let flags = DecoderFlags {
        sync_stream: true,
        json_mode: false,
    };
    assert_eq!(DECODER_FLAGS, flags);
    invoke_provider_selection();
    converse_and_other_endpoints();
    legacy_factory_has_no_moonshot_branch();
}

#[rstest]
#[case::invoke_id_encoding(invoke_id_encoding)]
#[case::converse_id_encoding(converse_id_encoding)]
#[case::bearer_selection(bearer_selection)]
#[case::native_reply_log(native_reply_log)]
#[case::text_run_sealing(text_run_sealing)]
#[case::event_stream_collector(event_stream_collector)]
#[case::passthrough_url(passthrough_url)]
#[case::plain_text_delta_rules(plain_text_delta_rules)]
#[case::collector_decoder_selection(collector_decoder_selection)]
fn passthrough_cases(#[case] case: fn()) {
    case();
}
