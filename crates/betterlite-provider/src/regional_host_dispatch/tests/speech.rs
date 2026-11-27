use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object as SpeechObject;
use http::HeaderMap;
use rstest::rstest;
use serde_json::{Value, json};

use crate::api::speech_provider_contract::{SpeechHeaders, SpeechRequestInput, TextToSpeechConfig};
use crate::regional_host_dispatch::error::DispatchError;
use crate::regional_host_dispatch::speech::{self, MinimaxSpeechConfig};

use super::support::{SpeechFixture, SpeechRecorder, params, secret};

const MISSING_KEY: &str = "MiniMax API key is required. Set MINIMAX_API_KEY environment variable or pass api_key parameter.";

fn voice_map(source: &str) -> SpeechVoice {
    SpeechVoice::Options(
        serde_json::from_str::<SpeechObject>(source).expect("the voice fixture parses"),
    )
}

// the arm's own voice step: voice_id, then id, then name, and a string voice passes as it is
#[rstest]
#[case::voice_id(r#"{"voice_id":"v-1","id":"v-2","name":"v-3"}"#, Some("v-1"))]
#[case::id(r#"{"id":"v-2","name":"v-3"}"#, Some("v-2"))]
#[case::name(r#"{"name":"v-3"}"#, Some("v-3"))]
#[case::empty_voice_id(r#"{"voice_id":"","name":"v-3"}"#, Some("v-3"))]
#[case::nothing("{}", None)]
#[tokio::test]
async fn minimax_speech_reads_the_voice_map_in_litellm_order(
    #[case] voice: &str,
    #[case] expected: Option<&str>,
) {
    let selected = MinimaxSpeechConfig { env_base: None };
    let config: &dyn TextToSpeechConfig = &selected;
    let mut fixture = SpeechFixture::new();
    fixture.voice = Some(voice_map(voice));
    let handler = SpeechRecorder::new();
    speech::minimax(Some(config), &fixture.context(), &handler)
        .await
        .expect("the arm answers");
    assert_eq!(handler.first().voice.as_deref(), expected);
}

#[tokio::test]
async fn minimax_speech_keeps_a_string_voice() {
    let selected = MinimaxSpeechConfig { env_base: None };
    let config: &dyn TextToSpeechConfig = &selected;
    let mut fixture = SpeechFixture::new();
    fixture.voice = Some(SpeechVoice::Name(String::from("female-shaonv")));
    let handler = SpeechRecorder::new();
    speech::minimax(Some(config), &fixture.context(), &handler)
        .await
        .expect("the arm answers");
    assert_eq!(handler.first().voice.as_deref(), Some("female-shaonv"));
}

// with no selected config the arm maps the voice and the options itself, as speech() would have
#[tokio::test]
async fn minimax_speech_maps_the_voice_without_a_selected_config() {
    let mut fixture = SpeechFixture::new();
    fixture.voice = Some(voice_map(r#"{"id":"v-2"}"#));
    fixture.optional_params = params(r#"{"speed":3.0}"#);
    let handler = SpeechRecorder::new();
    speech::minimax(None, &fixture.context(), &handler)
        .await
        .expect("the arm answers");
    let call = handler.first();
    assert_eq!(call.voice.as_deref(), Some("v-2"));
    assert_eq!(
        call.optional_params.get("voice_id"),
        Some(&Value::String(String::from("v-2")))
    );
    assert_eq!(call.optional_params.get("speed"), Some(&json!(2.0)));
    assert_eq!(
        call.optional_params.get("format"),
        Some(&Value::String(String::from("mp3")))
    );
}

#[tokio::test]
async fn minimax_speech_reads_the_voice_id_option_without_a_voice() {
    let mut fixture = SpeechFixture::new();
    fixture.optional_params = params(r#"{"voice_id":"v-9"}"#);
    let handler = SpeechRecorder::new();
    speech::minimax(None, &fixture.context(), &handler)
        .await
        .expect("the arm answers");
    let call = handler.first();
    assert_eq!(call.voice.as_deref(), Some("v-9"));
    assert_eq!(
        call.optional_params.get("voice_id"),
        Some(&Value::String(String::from("v-9")))
    );
}

#[tokio::test]
async fn minimax_speech_without_a_voice_uses_the_minimax_default() {
    let fixture = SpeechFixture::new();
    let handler = SpeechRecorder::new();
    speech::minimax(None, &fixture.context(), &handler)
        .await
        .expect("the arm answers");
    let call = handler.first();
    assert_eq!(call.voice.as_deref(), Some("male-qn-qingse"));
    assert_eq!(
        call.optional_params.get("voice_id"),
        Some(&Value::String(String::from("male-qn-qingse")))
    );
}

#[tokio::test]
async fn minimax_speech_with_a_selected_config_passes_the_options_unmapped() {
    let selected = MinimaxSpeechConfig { env_base: None };
    let config: &dyn TextToSpeechConfig = &selected;
    let mut fixture = SpeechFixture::new();
    fixture.voice = Some(SpeechVoice::Name(String::from("v-1")));
    fixture.optional_params = params(r#"{"response_format":"wav"}"#);
    let handler = SpeechRecorder::new();
    speech::minimax(Some(config), &fixture.context(), &handler)
        .await
        .expect("the arm answers");
    let call = handler.first();
    assert_eq!(
        call.optional_params.get("response_format"),
        Some(&Value::String(String::from("wav")))
    );
    assert_eq!(call.optional_params.len(), 1_usize);
}

#[rstest]
#[case::nothing(None, None, false, false)]
#[case::both(Some("https://tts.example"), Some("sk-request"), true, true)]
#[tokio::test]
async fn minimax_speech_writes_the_base_and_key_into_litellm_params_only_when_set(
    #[case] api_base: Option<&str>,
    #[case] api_key: Option<&str>,
    #[case] base_present: bool,
    #[case] key_present: bool,
) {
    let mut fixture = SpeechFixture::new();
    fixture.api_base = api_base.map(str::to_owned);
    fixture.api_key = api_key.map(secret);
    fixture.litellm_params = params(r#"{"user":"oracle"}"#);
    let handler = SpeechRecorder::new();
    speech::minimax(None, &fixture.context(), &handler)
        .await
        .expect("the arm answers");
    let call = handler.first();
    assert_eq!(
        call.litellm_params.get("user"),
        Some(&Value::String(String::from("oracle")))
    );
    assert_eq!(call.litellm_params.contains_key("api_base"), base_present);
    assert_eq!(call.litellm_params.contains_key("api_key"), key_present);
    assert_eq!(call.api_base.as_deref(), api_base);
    assert_eq!(
        call.litellm_params.get("api_key"),
        api_key.map(|key| Value::String(key.to_owned())).as_ref()
    );
}

#[rstest]
#[case::request_key(
    Some("sk-request"),
    Some("sk-global"),
    Some("sk-env"),
    Some("sk-request")
)]
#[case::global_key(None, Some("sk-global"), Some("sk-env"), Some("sk-global"))]
#[case::env_key(None, None, Some("sk-env"), Some("sk-env"))]
#[tokio::test]
async fn minimax_speech_key_chain(
    #[case] request_key: Option<&str>,
    #[case] global_key: Option<&str>,
    #[case] env_key: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut fixture = SpeechFixture::new();
    fixture.api_key = request_key.map(secret);
    fixture.settings.api_key = global_key.map(secret);
    fixture.settings.environment.minimax_api_key = env_key.map(secret);
    let handler = SpeechRecorder::new();
    speech::minimax(None, &fixture.context(), &handler)
        .await
        .expect("the arm answers");
    assert_eq!(handler.first().api_key.as_deref(), expected);
}

#[tokio::test]
async fn minimax_speech_without_a_key_raises_the_litellm_value_error() {
    let fixture = SpeechFixture::new();
    let handler = SpeechRecorder::new();
    let outcome = speech::minimax(None, &fixture.context(), &handler).await;
    assert!(matches!(
        outcome,
        Err(DispatchError::MissingKey { message }) if message == MISSING_KEY
    ));
    assert_eq!(handler.calls(), 0_usize);
}

#[tokio::test]
async fn minimax_speech_passes_the_request_fields_to_the_handler() {
    let mut fixture = SpeechFixture::new();
    fixture.model = String::from("speech-02-hd");
    fixture.input = String::from("hello there");
    fixture.api_key = Some(secret("sk-request"));
    fixture.ssl_verify = Some(false);
    fixture.timeout = Some(std::time::Duration::from_secs(7));
    let mut extra_headers = SpeechHeaders::new();
    extra_headers.insert(String::from("x-scope"), String::from("tenant"));
    fixture.extra_headers = Some(extra_headers);
    let handler = SpeechRecorder::new();
    speech::minimax(None, &fixture.context(), &handler)
        .await
        .expect("the arm answers");
    let call = handler.first();
    assert_eq!(call.model, "speech-02-hd");
    assert_eq!(call.input, "hello there");
    assert_eq!(call.ssl_verify, Some(false));
    assert_eq!(call.timeout, fixture.timeout);
    assert_eq!(call.extra_headers, fixture.extra_headers);
}

#[rstest]
#[case::argument_base(
    Some("https://arg.example"),
    Some("https://env.example"),
    "https://arg.example/v1/t2a_v2"
)]
#[case::environment_base(None, Some("https://env.example/"), "https://env.example/v1/t2a_v2")]
#[case::default_base(None, None, "https://api.minimax.io/v1/t2a_v2")]
fn minimax_config_completes_the_url(
    #[case] api_base: Option<&str>,
    #[case] env_base: Option<&str>,
    #[case] expected: &str,
) {
    let config = MinimaxSpeechConfig { env_base };
    let url = config
        .complete_url("speech-02-hd", api_base, &SpeechObject::new())
        .expect("the url builds");
    assert_eq!(url, expected);
}

#[test]
fn minimax_config_adds_the_bearer_and_content_type_headers() {
    let config = MinimaxSpeechConfig { env_base: None };
    let key = secret("sk-minimax");
    let headers =
        config.validate_environment(&SpeechHeaders::new(), "speech-02-hd", Some(&key), None);
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-minimax")
    );
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn minimax_config_builds_the_request_body_with_the_voice_setting() {
    let config = MinimaxSpeechConfig { env_base: None };
    let data = config.speech_request(&SpeechRequestInput {
        model: "speech-02-hd",
        input: "hello",
        voice: Some("female-shaonv"),
        optional_params: &SpeechObject::new(),
        litellm_params: &SpeechObject::new(),
        headers: &SpeechHeaders::new(),
    });
    let body = data.dict_body.expect("the request carries a dict body");
    assert_eq!(
        body.get("model"),
        Some(&Value::String(String::from("speech-02-hd")))
    );
    assert_eq!(
        body.get("text"),
        Some(&Value::String(String::from("hello")))
    );
    assert_eq!(
        body.get("voice_setting")
            .and_then(|setting| setting.get("voice_id")),
        Some(&Value::String(String::from("female-shaonv")))
    );
    assert_eq!(data.ssml_body, None);
    assert_eq!(
        data.headers
            .and_then(|headers| headers.get("Content-Type").cloned()),
        Some(String::from("application/json"))
    );
}

#[test]
fn minimax_config_decodes_the_hex_audio_reply() {
    let config = MinimaxSpeechConfig { env_base: None };
    let reply = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: br#"{"data":{"audio":"6869"}}"#.to_vec(),
    };
    let content = config.speech_response("speech-02-hd", reply);
    assert_eq!(content.status, 200);
    assert_eq!(content.body, b"hi");
}

// LiteLLM raises on a reply it cannot decode; this transform has no error channel, so the raw reply passes (docs/deviations/B8312.yaml)
#[test]
fn minimax_config_passes_a_reply_it_cannot_decode() {
    let config = MinimaxSpeechConfig { env_base: None };
    let reply = UpstreamResponse {
        status: 502,
        headers: HeaderMap::new(),
        body: b"not json".to_vec(),
    };
    let content = config.speech_response("speech-02-hd", reply);
    assert_eq!(content.status, 502);
    assert_eq!(content.body, b"not json");
}
