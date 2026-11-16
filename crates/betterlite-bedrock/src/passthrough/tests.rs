use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use betterlite_context::chat_config_selection::selection::ChatConfigError;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig;
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
use super::record::DecodedChunk;
use super::reply::{NativeReplyRequest, NativeReplyTransform};
use super::url::PassthroughEnvironment;
use super::{
    BedrockPassthroughPlan, CollectorClock, PassthroughIdentity, get_bedrock_bearer_token,
};

mod collecting;
mod runs;
mod safety;

use collecting::event_stream_collector;
use runs::{plain_text_delta_rules, text_run_sealing};

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
    fn transform(&self, request: &NativeReplyRequest<'_>) -> Result<Json, ProviderError> {
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
            return Err(ProviderError::from(ProviderErrorParts {
                status_code: 400,
                message: "the transform failed with FIXTURE-SECRET".to_owned(),
                headers: None,
                request: None,
                response: None,
                body: None,
                status_code_is_synthesized: false,
            }));
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
        *self.calls.lock().unwrap() += 1;
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
        now: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
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

fn bare_environment() -> PassthroughEnvironment {
    PassthroughEnvironment {
        region_name: None,
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

fn invoke_id_encoding() {
    let (selector, transform, assembler) = fixtures();
    let config = plan(&selector, &transform, &assembler, environment(), identity());
    let arn = object(
        r#"{"model_id":"arn:aws:bedrock:us-east-1:123:application-inference-profile/abc123"}"#,
    );
    let request = url_request(
        target("anthropic.claude-3", "model/anthropic.claude-3/invoke"),
        &arn,
        None,
    );
    assert_eq!(
        url_of(&config, &request),
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/arn:aws:bedrock:us-east-1:123:application-inference-profile%2Fabc123/invoke"
    );
    let slashed = object(r#"{"model_id":"custom/id"}"#);
    let request = url_request(
        target("anthropic.claude-3", "model/anthropic.claude-3/invoke"),
        &slashed,
        None,
    );
    assert_eq!(
        url_of(&config, &request),
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/custom/invoke"
    );
    assert_eq!(encoded_model_id(""), "");
    assert_eq!(encoded_model_id("/model/a"), "a");
    assert_eq!(
        encoded_model_id("arn:aws:bedrock:eu-west-1:1:foundation-model/x"),
        "arn:aws:bedrock:eu-west-1:1:foundation-model%2Fx"
    );
}

fn converse_id_encoding() {
    let (selector, transform, assembler) = fixtures();
    let config = plan(&selector, &transform, &assembler, environment(), identity());
    let missing = object("{}");
    let request = url_request(
        target("anthropic.claude-3", "model/anthropic.claude-3/converse"),
        &missing,
        None,
    );
    assert_eq!(
        url_of(&config, &request),
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-3/converse"
    );
    let null = object(r#"{"model_id":null}"#);
    let request = url_request(
        target("anthropic.claude-3", "model/anthropic.claude-3/converse"),
        &null,
        None,
    );
    assert_eq!(
        url_of(&config, &request),
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-3/converse"
    );
    let empty = object(r#"{"model_id":""}"#);
    let request = url_request(
        target("anthropic.claude-3", "model/anthropic.claude-3/converse"),
        &empty,
        None,
    );
    assert_eq!(
        url_of(&config, &request),
        "https://bedrock-runtime.us-east-1.amazonaws.com/model//converse"
    );
    let percent = object(r#"{"model_id":"abc%2Fdef"}"#);
    let request = url_request(
        target("anthropic.claude-3", "model/anthropic.claude-3/converse"),
        &percent,
        None,
    );
    assert_eq!(
        url_of(&config, &request),
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/abc%2Fdef/converse"
    );
    let unmatched = object(r#"{"model_id":"replacement"}"#);
    let request = url_request(
        target("anthropic.claude-3", "custom/converse"),
        &unmatched,
        None,
    );
    assert_eq!(
        url_of(&config, &request),
        "https://bedrock-runtime.us-east-1.amazonaws.com/custom/converse"
    );
    assert_eq!(with_model_id("model/a/model/b/x", "z"), "model/z/model/z/x");
    assert_eq!(with_model_id("model//x", "z"), "model//x");
    assert_eq!(with_model_id("model///x", "z"), "model///x");
    let wrong = object(r#"{"model_id":5}"#);
    let failure = config
        .get_complete_url(&url_request(
            target("anthropic.claude-3", "model/x/converse"),
            &wrong,
            None,
        ))
        .unwrap_err();
    assert_eq!(failure.to_string(), "the field model_id is not a string");
}

fn bearer_selection() {
    let (selector, transform, assembler) = fixtures();
    let mut bearer_identity = identity();
    bearer_identity.bearer_token = Some(SecretString::from("fixture-bearer".to_owned()));
    let config = plan(
        &selector,
        &transform,
        &assembler,
        environment(),
        bearer_identity,
    );
    let params = object(r#"{"api_key":"sk-should-not-sign"}"#);
    let data = object(r#"{"input":"hi"}"#);
    let mut headers = Headers::new();
    headers.insert("x-custom".to_owned(), "kept".to_owned());
    let signing = PassthroughSigning {
        litellm_params: &params,
        request_data: Some(&data),
        api_base: "https://bedrock-runtime.us-east-1.amazonaws.com/model/m/converse",
        model: Some("m"),
    };
    let (signed, body) = config.sign_request(headers, &signing).unwrap();
    assert_eq!(
        signed.get("Authorization").map(String::as_str),
        Some("Bearer fixture-bearer")
    );
    assert_eq!(
        signed.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(signed.get("x-custom").map(String::as_str), Some("kept"));
    assert!(
        !signed
            .keys()
            .any(|name| name.eq_ignore_ascii_case("content-length"))
    );
    assert_eq!(
        String::from_utf8(body.unwrap()).unwrap(),
        r#"{"input": "hi"}"#
    );
    assert_eq!(get_bedrock_bearer_token(&params), None);

    let sigv4_config = plan(&selector, &transform, &assembler, environment(), identity());
    let signed_params = object(r#"{"aws_region_name":"eu-west-1"}"#);
    let (sigv4, signed_body) = sigv4_config
        .sign_request(
            Headers::new(),
            &PassthroughSigning {
                litellm_params: &signed_params,
                request_data: None,
                api_base: "https://bedrock-runtime.us-east-1.amazonaws.com/model/m/converse",
                model: Some("m"),
            },
        )
        .unwrap();
    let authorization = sigv4.get("Authorization").unwrap();
    assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIAFIXTURE/"));
    assert!(authorization.contains("/eu-west-1/bedrock/aws4_request"));
    assert_eq!(
        sigv4.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert!(
        !sigv4
            .keys()
            .any(|name| name.eq_ignore_ascii_case("content-length"))
    );
    assert_eq!(String::from_utf8(signed_body.unwrap()).unwrap(), "{}");

    let mut supplied = Headers::new();
    supplied.insert("Content-Type".to_owned(), "text/plain".to_owned());
    let (kept, _) = sigv4_config
        .sign_request(
            supplied,
            &PassthroughSigning {
                litellm_params: &signed_params,
                request_data: None,
                api_base: "https://bedrock-runtime.us-east-1.amazonaws.com/model/m/converse",
                model: Some("m"),
            },
        )
        .unwrap();
    assert_eq!(
        kept.get("Content-Type").map(String::as_str),
        Some("text/plain")
    );
    assert_eq!(
        kept.keys()
            .filter(|name| name.eq_ignore_ascii_case("content-type"))
            .count(),
        1
    );

    let mut error_headers = Headers::new();
    error_headers.insert("x-amzn-requestid".to_owned(), "abc".to_owned());
    let error = config.get_error_class("denied", 403, &error_headers);
    assert_eq!(error.status_code, 403);
    assert_eq!(error.message, "denied");
    assert!(!error.status_code_is_synthesized);
    assert_eq!(
        error.headers,
        Some(vec![("x-amzn-requestid".to_owned(), "abc".to_owned())])
    );
    let bare = config.get_error_class("denied", 403, &Headers::new());
    assert_eq!(bare.headers, None);
}

fn native_reply_log() {
    let (selector, transform, assembler) = fixtures();
    let config = plan(&selector, &transform, &assembler, environment(), identity());
    let request_data = object(r#"{"input":"hi"}"#);
    let response = RelayedResponse {
        status_code: 200,
        body: b"{\"id\":\"native\"}",
    };
    let mut logging = TestLogging;
    let logged = config
        .logging_non_streaming_response(
            &target("anthropic.claude-3", "model/anthropic.claude-3/invoke"),
            &response,
            &request_data,
            &mut logging,
        )
        .unwrap();
    assert_eq!(logged, json(r#"{"id":"logged"}"#));
    {
        let recorded = transform.recorded.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].tag, ChatConfigTag::Anthropic);
        assert_eq!(recorded[0].model, "anthropic.claude-3");
        assert_eq!(recorded[0].provider, "bedrock");
        assert_eq!(recorded[0].status, 200);
        assert_eq!(recorded[0].body, b"{\"id\":\"native\"}");
        assert_eq!(
            recorded[0].messages,
            vec![(
                "user".to_owned(),
                "no-message-pass-through-endpoint".to_owned()
            )]
        );
        assert!(recorded[0].optional_empty);
        assert!(recorded[0].litellm_empty);
        assert_eq!(recorded[0].api_key, "");
        assert_eq!(recorded[0].request_data, request_data);
    }
    assert_eq!(
        selector.calls.lock().unwrap()[0].0,
        "invoke/anthropic.claude-3"
    );
    assert_eq!(selector.calls.lock().unwrap()[0].1, ProviderId::Bedrock);
    config
        .logging_non_streaming_response(
            &target("anthropic.claude-3", "model/x/converse"),
            &response,
            &request_data,
            &mut logging,
        )
        .unwrap();
    assert_eq!(
        transform.recorded.lock().unwrap()[1].tag,
        ChatConfigTag::Anthropic
    );
    assert_eq!(
        selector.calls.lock().unwrap()[1].0,
        "converse/anthropic.claude-3"
    );
    config
        .logging_non_streaming_response(
            &target("anthropic.claude-3", "model/x/invoke/converse"),
            &response,
            &request_data,
            &mut logging,
        )
        .unwrap();
    assert_eq!(
        selector.calls.lock().unwrap()[2].0,
        "invoke/anthropic.claude-3"
    );
    assert!(
        config
            .logging_non_streaming_response(
                &target("anthropic.claude-3", "model/x"),
                &response,
                &request_data,
                &mut logging,
            )
            .is_none()
    );
    assert_eq!(transform.recorded.lock().unwrap().len(), 3);
    assert_eq!(selector.calls.lock().unwrap().len(), 3);
    let no_config = TestSelector {
        tag: None,
        calls: Mutex::new(Vec::new()),
    };
    let missing = plan(
        &no_config,
        &transform,
        &assembler,
        environment(),
        identity(),
    );
    let failure = super::reply::logged_reply(
        &missing,
        &target("anthropic.claude-3", "model/x/invoke"),
        &response,
        &request_data,
    )
    .unwrap_err();
    assert_eq!(
        failure.to_string(),
        "No provider config found for model: anthropic.claude-3"
    );
    let unknown = super::reply::logged_reply(
        &missing,
        &PassthroughTarget {
            model: "m",
            custom_llm_provider: "not-a-provider",
            endpoint: "model/m/invoke",
        },
        &response,
        &request_data,
    )
    .unwrap_err();
    assert_eq!(
        unknown.to_string(),
        "the provider not-a-provider is not a known provider"
    );
    let mut failing = fixtures().1;
    failing.fail = true;
    let failing_plan = plan(&selector, &failing, &assembler, environment(), identity());
    assert!(
        failing_plan
            .logging_non_streaming_response(
                &target("anthropic.claude-3", "model/x/invoke"),
                &response,
                &request_data,
                &mut logging,
            )
            .is_none()
    );
}

fn passthrough_url() {
    let (selector, transform, assembler) = fixtures();
    let config = plan(&selector, &transform, &assembler, environment(), identity());
    let params = object("{}");
    let request = url_request(
        target("anthropic.claude-3", "model/x/converse"),
        &params,
        Some("https://proxy.example/base/"),
    );
    assert_eq!(
        url_of(&config, &request),
        "https://proxy.example/base/model/x/converse"
    );
    let configured = object(r#"{"aws_bedrock_runtime_endpoint":"https://configured.example"}"#);
    let request = url_request(
        target("anthropic.claude-3", "model/x/converse"),
        &configured,
        None,
    );
    assert_eq!(
        url_of(&config, &request),
        "https://configured.example/model/x/converse"
    );
    let mut environed = environment();
    environed.runtime_endpoint = Some("https://env.example".to_owned());
    let config = plan(&selector, &transform, &assembler, environed, identity());
    let request = url_request(
        target("anthropic.claude-3", "model/x/converse"),
        &params,
        None,
    );
    assert_eq!(
        url_of(&config, &request),
        "https://env.example/model/x/converse"
    );
    let query = object(r#"{"beta":"1"}"#);
    let request = PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: target("anthropic.claude-3", "model/x/converse"),
        request_query_params: Some(&query),
        litellm_params: &params,
    };
    assert_eq!(
        url_of(&config, &request),
        "https://env.example/model/x/converse?beta=1"
    );
    let bare = bare_environment();
    let config = plan(&selector, &transform, &assembler, bare, identity());
    let request = url_request(
        target(
            "arn:aws:bedrock:eu-west-1:1:foundation-model/anthropic.claude",
            "model/x/converse",
        ),
        &params,
        None,
    );
    assert_eq!(
        url_of(&config, &request),
        "https://bedrock-runtime.eu-west-1.amazonaws.com/model/x/converse"
    );
    let explicit = object(r#"{"aws_region_name":"ap-south-1"}"#);
    let request = url_request(
        target(
            "arn:aws:bedrock:eu-west-1:1:foundation-model/anthropic.claude",
            "model/x/converse",
        ),
        &explicit,
        None,
    );
    assert_eq!(
        url_of(&config, &request),
        "https://bedrock-runtime.ap-south-1.amazonaws.com/model/x/converse"
    );
    let profile =
        object(r#"{"model_id":"arn:aws:bedrock:ap-south-1:1:application-inference-profile/x"}"#);
    let request = url_request(
        target("anthropic.claude-3", "model/x/converse"),
        &profile,
        None,
    );
    assert_eq!(
        url_of(&config, &request),
        "https://bedrock-runtime.ap-south-1.amazonaws.com/model/arn:aws:bedrock:ap-south-1:1:application-inference-profile%2Fx/converse"
    );
    let region = object(r#"{"aws_region_name":5}"#);
    let failure = config
        .get_complete_url(&url_request(
            target("anthropic.claude-3", "model/x/converse"),
            &region,
            None,
        ))
        .unwrap_err();
    assert_eq!(
        failure.to_string(),
        "the field aws_region_name is not a string"
    );
    assert!(config.is_streaming_request("model/x/stream", &params));
    assert!(config.is_streaming_request("model/x/converse-stream", &params));
    assert!(!config.is_streaming_request("model/x/invoke", &object(r#"{"stream":true}"#)));
    assert!(!config.is_streaming_request("model/x/STREAM", &params));
}

fn collector_decoder_selection() {
    assert_eq!(
        DECODER_FLAGS,
        DecoderFlags {
            sync_stream: true,
            json_mode: false,
        }
    );
    let invoke = decoder_selection("anthropic.claude-3", "model/x/invoke", clock());
    let DecoderChoice::Invoke { provider, .. } = invoke else {
        panic!("expected an invoke decoder");
    };
    assert_eq!(provider.name(), "anthropic");
    let missing = decoder_selection("mystery.v9", "model/x/invoke", clock());
    let DecoderChoice::Disabled { warning } = missing else {
        panic!("expected a disabled decoder");
    };
    assert_eq!(
        warning.as_deref(),
        Some("Bedrock passthrough spend tracking skipped: no invoke provider for model mystery.v9")
    );
    assert!(decoder_box(DecoderChoice::Disabled { warning: None }).is_none());
    let converse = decoder_selection("mystery.v9", "model/x/converse", clock());
    let DecoderChoice::Converse(mut converse_decoder) = converse else {
        panic!("expected a converse decoder");
    };
    let nova: Json = json(r#"{"contentBlockDelta":{"delta":{"text":"hi"}}}"#);
    let chunk = converse_decoder.decode(&nova, clock()).unwrap();
    let DecodedChunk::Chunk(chunk) = chunk else {
        panic!("expected a translated chunk");
    };
    assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hi"));
    assert_eq!(chunk.model, "mystery.v9");
    let other = decoder_selection("mystery.v9", "model/x/other", clock());
    let DecoderChoice::Disabled { warning } = other else {
        panic!("expected a disabled decoder");
    };
    assert_eq!(warning, None);
    let both = decoder_selection("anthropic.claude-3", "model/x/invoke/converse", clock());
    assert!(matches!(both, DecoderChoice::Invoke { .. }));
    let moonshot = decoder_selection("moonshot.v1", "model/moonshot.v1/invoke", clock());
    let DecoderChoice::Invoke {
        provider,
        mut decoder,
    } = moonshot
    else {
        panic!("expected an invoke decoder");
    };
    assert_eq!(provider.name(), "moonshot");
    let event: Json = json(r#"{"contentBlockDelta":{"delta":{"text":"hi"}}}"#);
    let chunk = decoder.decode(&event, clock()).unwrap();
    let DecodedChunk::Chunk(chunk) = chunk else {
        panic!("expected a translated chunk");
    };
    assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hi"));
    let converse_boxed = decoder_box(decoder_selection(
        "anthropic.claude-3",
        "model/x/converse",
        clock(),
    ));
    assert!(converse_boxed.is_some());
    let stop: Json = json(r#"{"stopReason":"end_turn"}"#);
    let mut boxed = converse_boxed.unwrap();
    let chunk = boxed.decode(&stop, clock()).unwrap();
    let DecodedChunk::Chunk(chunk) = chunk else {
        panic!("expected a translated chunk");
    };
    assert_eq!(chunk.choices[0].finish_reason.as_deref(), Some("stop"));
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
