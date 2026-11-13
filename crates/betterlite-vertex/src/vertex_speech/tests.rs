#![cfg(test)]

use std::fs;
use std::future::{Future, ready};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Mutex;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::audio::{BinaryAudio, decode_vertex_speech_audio};
use super::error::{Base64Error, VertexSpeechError};
use super::mapping::{
    DEFAULT_SPEAKING_RATE, SpeechAudioConfig, SpeechKwargs, SpeechMapping, SpeechOptionalParams,
    SpeechVoice, map_openai_params, voice_language,
};
use super::request::{
    SpeechCall, SpeechInput, SpeechRequest, SpeechVoiceSettings, TTS_API_URL, complete_url,
    prepare_request, validate_environment, validate_input,
};
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::token_cache::probe::AccessToken;
use crate::vertex_interactions::mint::{MintAccessToken, MintFuture};

struct FixedMint {
    token: &'static str,
    project: &'static str,
    calls: Mutex<Vec<(Option<String>, Option<String>)>>,
}

impl FixedMint {
    fn new() -> Self {
        Self {
            token: "test-token",
            project: "project-1",
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(Option<String>, Option<String>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl MintAccessToken for FixedMint {
    fn mint<'mint>(
        &'mint self,
        credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        self.calls.lock().unwrap().push((
            credentials
                .and_then(ConfiguredCredentials::as_text)
                .map(str::to_owned),
            project.map(str::to_owned),
        ));
        let resolved = project.map_or_else(|| self.project.to_owned(), str::to_owned);
        Box::pin(ready(Ok(AccessToken {
            token: SecretString::from(self.token),
            project: resolved,
        })))
    }
}

fn block_on<Call, Output>(future: Call) -> Output
where
    Call: Future<Output = Output>,
{
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn settings_object(language: &str, name: &str) -> Object {
    let mut object = Object::new();
    object.insert("languageCode".to_owned(), Json::Text(language.to_owned()));
    object.insert("name".to_owned(), Json::Text(name.to_owned()));
    object
}

fn mapping(optional: SpeechOptionalParams, kwargs: SpeechKwargs) -> SpeechMapping {
    map_openai_params(None, &optional, &kwargs)
}

fn named_voice(name: &str) -> SpeechVoice {
    SpeechVoice::Name(name.to_owned())
}

fn call<'call>(input: &'call str, mapping: &'call SpeechMapping) -> SpeechCall<'call> {
    SpeechCall {
        input,
        voice: None,
        mapping,
        voice_dict: None,
        credentials: None,
        project: None,
        location: None,
    }
}

fn request_for(speech_call: &SpeechCall<'_>) -> Result<SpeechRequest, VertexSpeechError> {
    let mint = FixedMint::new();
    block_on(prepare_request(&mint, speech_call, &IndexMap::new()))
}

fn sniff_x(audio: &[u8]) -> Option<String> {
    (audio.first() == Some(&b'X')).then_some("audio/x-test".to_owned())
}

fn decode(body: &str) -> Result<BinaryAudio, VertexSpeechError> {
    decode_vertex_speech_audio(body.as_bytes(), &sniff_x)
}

#[rstest]
#[case::alloy("alloy", "en-US-Studio-O")]
#[case::echo("echo", "en-US-Studio-M")]
#[case::fable("fable", "en-GB-Studio-B")]
#[case::onyx("onyx", "en-US-Wavenet-D")]
#[case::nova("nova", "en-US-Studio-O")]
#[case::shimmer("shimmer", "en-US-Wavenet-F")]
fn maps_every_openai_voice_to_its_vertex_voice(#[case] voice: &str, #[case] mapped: &str) {
    let mapped_params = map_openai_params(
        Some(&named_voice(voice)),
        &SpeechOptionalParams::default(),
        &SpeechKwargs::default(),
    );
    assert_eq!(mapped_params.voice.as_deref(), Some(voice));
    assert_eq!(
        mapped_params.voice_dict,
        Some(settings_object(&voice_language(mapped), mapped))
    );
}

#[rstest]
#[case::vertex("en-US-Wavenet-D", "en-US")]
#[case::german("de-DE-Standard-A", "de-DE")]
#[case::unknown("custom", "en-US")]
#[case::single_hyphen("-x", "-x")]
#[case::empty_region("en--US", "en-")]
#[case::three_parts("a-b-c", "a-b")]
#[case::empty("", "en-US")]
fn uses_a_vertex_voice_name_as_it_stands(#[case] voice: &str, #[case] language: &str) {
    let mapped_params = map_openai_params(
        Some(&named_voice(voice)),
        &SpeechOptionalParams::default(),
        &SpeechKwargs::default(),
    );
    assert_eq!(
        mapped_params.voice_dict,
        Some(settings_object(language, voice))
    );
}

#[rstest]
#[case::studio("en-US-Studio-O", "en-US")]
#[case::two_components("de-DE", "de-DE")]
#[case::single("custom", "en-US")]
#[case::empty_region("en--US", "en-")]
#[case::empty("", "en-US")]
fn extracts_the_language_from_the_first_two_components(#[case] name: &str, #[case] language: &str) {
    assert_eq!(voice_language(name), language);
}

#[test]
fn keeps_a_voice_dictionary_unchanged() {
    let mut options = Object::new();
    options.insert("name".to_owned(), Json::Text("custom".to_owned()));
    options.insert("extra".to_owned(), Json::Number(1_u64.into()));
    let mapped_params = map_openai_params(
        Some(&SpeechVoice::Options(options.clone())),
        &SpeechOptionalParams::default(),
        &SpeechKwargs::default(),
    );
    assert_eq!(mapped_params.voice, None);
    assert_eq!(mapped_params.voice_dict, Some(options));
}

#[rstest]
#[case::mp3("mp3", "MP3")]
#[case::aac_is_mp3("aac", "MP3")]
#[case::opus("opus", "OGG_OPUS")]
#[case::flac("flac", "FLAC")]
#[case::wav("wav", "LINEAR16")]
#[case::pcm("pcm", "LINEAR16")]
#[case::unknown_passes_through("ogg", "ogg")]
#[case::empty_passes_through("", "")]
fn maps_a_response_format_to_its_audio_encoding(#[case] format: &str, #[case] encoding: &str) {
    let optional = SpeechOptionalParams {
        response_format: Some(format.to_owned()),
        speed: None,
    };
    assert_eq!(
        mapping(optional, SpeechKwargs::default()).audio_encoding,
        encoding
    );
}

#[test]
fn defaults_to_linear16_without_a_response_format() {
    assert_eq!(
        mapping(SpeechOptionalParams::default(), SpeechKwargs::default()).audio_encoding,
        "LINEAR16"
    );
}

#[rstest]
#[case::whole(2, "2")]
#[case::zero(0, "0")]
#[case::negative(-1, "-1")]
fn stringifies_a_non_none_speed(#[case] speed: i64, #[case] rate: &str) {
    let optional = SpeechOptionalParams {
        response_format: None,
        speed: Some(speed),
    };
    assert_eq!(
        mapping(optional, SpeechKwargs::default())
            .speaking_rate
            .as_deref(),
        Some(rate)
    );
}

#[test]
fn leaves_the_speaking_rate_unset_without_a_speed() {
    assert_eq!(
        mapping(SpeechOptionalParams::default(), SpeechKwargs::default()).speaking_rate,
        None
    );
}

#[test]
fn copies_a_full_audio_config_and_use_ssml_from_the_kwargs() {
    let audio_config = SpeechAudioConfig {
        audio_encoding: Some("MP3".to_owned()),
        speaking_rate: Some("2".to_owned()),
    };
    let kwargs = SpeechKwargs {
        audio_config: Some(audio_config.clone()),
        use_ssml: true,
    };
    let mapped_params = mapping(SpeechOptionalParams::default(), kwargs);
    assert_eq!(mapped_params.audio_config, Some(audio_config));
    assert!(mapped_params.use_ssml);
    assert_eq!(mapped_params.audio_encoding, "LINEAR16");
}

#[test]
fn leaves_the_audio_config_unset_without_the_kwarg() {
    assert_eq!(
        mapping(SpeechOptionalParams::default(), SpeechKwargs::default()).audio_config,
        None
    );
}

fn validated(
    text: Option<&str>,
    ssml: Option<&str>,
    use_ssml: bool,
) -> Result<SpeechInput, VertexSpeechError> {
    let input = SpeechInput {
        text: text.map(str::to_owned),
        ssml: ssml.map(str::to_owned),
    };
    validate_input(&input, use_ssml)
}

#[rstest]
#[case::plain("hello", false, Some("hello"), None)]
#[case::speak_substring("<speak>hi</speak>", false, None, Some("<speak>hi</speak>"))]
#[case::speak_inside_text("a <speak> b", false, None, Some("a <speak> b"))]
#[case::uppercase_is_not_ssml("<SPEAK>hi</SPEAK>", false, Some("<SPEAK>hi</SPEAK>"), None)]
#[case::unfinished_tag("<speak", false, Some("<speak"), None)]
#[case::empty_text("", false, Some(""), None)]
#[case::forced_ssml("hi", true, None, Some("hi"))]
#[case::forced_empty_ssml("", true, None, Some(""))]
fn moves_a_text_input_into_ssml(
    #[case] text: &str,
    #[case] use_ssml: bool,
    #[case] expected_text: Option<&str>,
    #[case] expected_ssml: Option<&str>,
) {
    let input = validated(Some(text), None, use_ssml).unwrap();
    assert_eq!(input.text.as_deref(), expected_text);
    assert_eq!(input.ssml.as_deref(), expected_ssml);
}

#[test]
fn keeps_an_existing_ssml_input() {
    let input = validated(None, Some("<speak>hi</speak>"), true).unwrap();
    assert_eq!(
        input,
        SpeechInput {
            text: None,
            ssml: Some("<speak>hi</speak>".to_owned()),
        }
    );
}

#[test]
fn replaces_an_existing_ssml_with_a_speak_text() {
    let input = validated(Some("<speak>x"), Some("y"), false).unwrap();
    assert_eq!(
        input,
        SpeechInput {
            text: None,
            ssml: Some("<speak>x".to_owned()),
        }
    );
}

#[test]
fn rejects_use_ssml_without_any_input() {
    assert!(matches!(
        validated(None, None, true),
        Err(VertexSpeechError::SsmlRequired)
    ));
}

#[test]
fn rejects_absent_input() {
    assert!(matches!(
        validated(None, None, false),
        Err(VertexSpeechError::InputRequired)
    ));
}

#[test]
fn rejects_both_text_and_ssml() {
    assert!(matches!(
        validated(Some("x"), Some("y"), false),
        Err(VertexSpeechError::BothInputs)
    ));
}

#[test]
fn copies_caller_headers_and_forces_json_and_utf8() {
    let mut caller = IndexMap::new();
    caller.insert("Authorization".to_owned(), "caller-secret".to_owned());
    caller.insert("Content-Type".to_owned(), "text/plain".to_owned());
    caller.insert("charset".to_owned(), "latin-1".to_owned());
    let headers = validate_environment(&caller);
    assert_eq!(headers["Content-Type"].expose_secret(), "application/json");
    assert_eq!(headers["charset"].expose_secret(), "UTF-8");
    assert_eq!(headers["Authorization"].expose_secret(), "caller-secret");
    assert_eq!(caller["Content-Type"], "text/plain");
}

#[rstest]
#[case::default_endpoint(None, TTS_API_URL)]
#[case::empty_base("", TTS_API_URL)]
#[case::custom_base("https://host/v1/text:synthesize", "https://host/v1/text:synthesize")]
fn resolves_the_synthesis_url(#[case] api_base: Option<&str>, #[case] url: &str) {
    assert_eq!(complete_url(api_base), url);
}

#[test]
fn defaults_to_the_studio_voice() {
    let mapped = mapping(SpeechOptionalParams::default(), SpeechKwargs::default());
    let request = request_for(&call("hi", &mapped)).unwrap();
    assert_eq!(
        request.body.voice,
        SpeechVoiceSettings {
            language_code: Some("en-US".to_owned()),
            name: Some("en-US-Studio-O".to_owned()),
        }
    );
}

#[test]
fn prefers_a_truthy_litellm_voice_dict_over_the_mapped_one() {
    let mapped = map_openai_params(
        Some(&named_voice("alloy")),
        &SpeechOptionalParams::default(),
        &SpeechKwargs::default(),
    );
    let mut litellm_dict = Object::new();
    litellm_dict.insert("name".to_owned(), Json::Text("de-DE-Wavenet-A".to_owned()));
    let mut speech_call = call("hi", &mapped);
    speech_call.voice_dict = Some(&litellm_dict);
    let request = request_for(&speech_call).unwrap();
    assert_eq!(request.body.voice.name.as_deref(), Some("de-DE-Wavenet-A"));
    assert_eq!(request.body.voice.language_code, None);
}

#[test]
fn falls_back_to_the_mapped_voice_dict() {
    let mapped = map_openai_params(
        Some(&named_voice("alloy")),
        &SpeechOptionalParams::default(),
        &SpeechKwargs::default(),
    );
    let empty = Object::new();
    let mut speech_call = call("hi", &mapped);
    speech_call.voice_dict = Some(&empty);
    let request = request_for(&speech_call).unwrap();
    assert_eq!(request.body.voice.name.as_deref(), Some("en-US-Studio-O"));
}

#[test]
fn keeps_an_empty_mapped_voice_dict_empty() {
    let mapped = map_openai_params(
        Some(&SpeechVoice::Options(Object::new())),
        &SpeechOptionalParams::default(),
        &SpeechKwargs::default(),
    );
    let request = request_for(&call("hi", &mapped)).unwrap();
    assert_eq!(request.body.voice, SpeechVoiceSettings::default());
}

#[test]
fn rejects_a_non_string_voice_field() {
    let mut options = Object::new();
    options.insert("name".to_owned(), Json::Number(5_u64.into()));
    let mapped = map_openai_params(
        Some(&SpeechVoice::Options(options)),
        &SpeechOptionalParams::default(),
        &SpeechKwargs::default(),
    );
    assert!(matches!(
        request_for(&call("hi", &mapped)),
        Err(VertexSpeechError::VoiceFieldType { .. })
    ));
}

#[test]
fn builds_the_default_audio_config() {
    let mapped = mapping(SpeechOptionalParams::default(), SpeechKwargs::default());
    let request = request_for(&call("hi", &mapped)).unwrap();
    assert_eq!(
        request.body.audio_config,
        SpeechAudioConfig {
            audio_encoding: Some("LINEAR16".to_owned()),
            speaking_rate: Some(DEFAULT_SPEAKING_RATE.to_owned()),
        }
    );
}

#[test]
fn lets_a_full_audio_config_replace_the_encoding_and_rate() {
    let kwargs = SpeechKwargs {
        audio_config: Some(SpeechAudioConfig {
            audio_encoding: None,
            speaking_rate: Some("2".to_owned()),
        }),
        use_ssml: false,
    };
    let mapped = mapping(
        SpeechOptionalParams {
            response_format: Some("mp3".to_owned()),
            speed: Some(4),
        },
        kwargs,
    );
    let request = request_for(&call("hi", &mapped)).unwrap();
    assert_eq!(
        request.body.audio_config,
        SpeechAudioConfig {
            audio_encoding: None,
            speaking_rate: Some("2".to_owned()),
        }
    );
}

#[test]
fn overwrites_authorization_and_the_project_header() {
    let mapped = mapping(SpeechOptionalParams::default(), SpeechKwargs::default());
    let mut caller = IndexMap::new();
    caller.insert("Authorization".to_owned(), "caller-secret".to_owned());
    let mint = FixedMint::new();
    let credentials = ConfiguredCredentials::from_text("creds".to_owned()).unwrap();
    let mut speech_call = call("hi", &mapped);
    speech_call.credentials = Some(&credentials);
    speech_call.project = Some("project-2");
    let request = block_on(prepare_request(&mint, &speech_call, &caller)).unwrap();
    assert_eq!(
        request.headers["Authorization"].expose_secret(),
        "Bearer test-token"
    );
    assert_eq!(
        request.headers["x-goog-user-project"].expose_secret(),
        "project-2"
    );
    assert_eq!(
        mint.calls(),
        vec![(Some("creds".to_owned()), Some("project-2".to_owned()))]
    );
}

#[test]
fn redacts_header_values_in_debug() {
    let mapped = mapping(SpeechOptionalParams::default(), SpeechKwargs::default());
    let mut caller = IndexMap::new();
    caller.insert("x-api-key".to_owned(), "caller-secret".to_owned());
    let mint = FixedMint::new();
    let request = block_on(prepare_request(&mint, &call("hi", &mapped), &caller)).unwrap();
    let rendered = format!("{request:?}");
    assert!(!rendered.contains("test-token"));
    assert!(!rendered.contains("caller-secret"));
}

#[test]
fn rejects_an_invalid_vertex_location() {
    let mapped = mapping(SpeechOptionalParams::default(), SpeechKwargs::default());
    let mut speech_call = call("hi", &mapped);
    speech_call.location = Some("Bad_Location");
    assert!(matches!(
        request_for(&speech_call),
        Err(VertexSpeechError::Location(_))
    ));
}

#[test]
fn decodes_audio_and_sniffs_its_type() {
    let audio = decode(r#"{"audioContent": "WFk="}"#).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"XY");
    assert_eq!(audio.content_type.as_deref(), Some("audio/x-test"));
    assert_eq!(audio.status, 200);
}

#[test]
fn leaves_the_content_type_off_when_nothing_matches() {
    let audio = decode(r#"{"audioContent": "QUJD"}"#).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"ABC");
    assert_eq!(audio.content_type, None);
    assert_eq!(audio.status, 200);
}

#[rstest]
#[case::empty_object("{}")]
#[case::null_content(r#"{"audioContent": null}"#)]
#[case::empty_content(r#"{"audioContent": ""}"#)]
#[case::zero_content(r#"{"audioContent": 0}"#)]
#[case::false_content(r#"{"audioContent": false}"#)]
#[case::empty_list(r#"{"audioContent": []}"#)]
#[case::empty_map(r#"{"audioContent": {}}"#)]
fn rejects_a_falsey_audio_content(#[case] body: &str) {
    assert!(matches!(
        decode(body),
        Err(VertexSpeechError::MissingAudioContent)
    ));
}

#[rstest]
#[case::number(r#"{"audioContent": 5}"#)]
#[case::truthy_list(r#"{"audioContent": [1]}"#)]
#[case::truthy_map(r#"{"audioContent": {"a": 1}}"#)]
fn rejects_a_non_text_audio_content(#[case] body: &str) {
    assert!(matches!(
        decode(body),
        Err(VertexSpeechError::AudioContentNotText)
    ));
}

#[test]
fn rejects_a_non_ascii_audio_content() {
    assert!(matches!(
        decode("{\"audioContent\": \"\u{e9}\"}"),
        Err(VertexSpeechError::AudioContentNotAscii)
    ));
}

#[rstest]
#[case::junk_is_skipped(r#"{"audioContent": "!!!"}"#, &b""[..])]
#[case::full_quad(r#"{"audioContent": "QUJD"}"#, &b"ABC"[..])]
#[case::padded_quad(r#"{"audioContent": "QQ=="}"#, &b"A"[..])]
#[case::padding_inside(r#"{"audioContent": "QQ==QUJDRA=="}"#, &b"A\x04\x14$4@"[..])]
#[case::spaces_are_skipped(r#"{"audioContent": "Q Q = ="}"#, &b"A"[..])]
#[case::newlines_are_skipped(r#"{"audioContent": "QUJD\nRA=="}"#, &b"ABCD"[..])]
#[case::extra_padding(r#"{"audioContent": "AAAA==="}"#, &b"\x00\x00\x00"[..])]
fn decodes_permissive_base64(#[case] body: &str, #[case] expected: &[u8]) {
    assert_eq!(decode(body).unwrap().bytes.as_slice(), expected);
}

#[rstest]
#[case::no_padding(r#"{"audioContent": "QQ"}"#)]
#[case::short_group(r#"{"audioContent": "QUJDRA"}"#)]
fn rejects_base64_without_final_padding(#[case] body: &str) {
    assert!(matches!(
        decode(body),
        Err(VertexSpeechError::Base64(Base64Error::MissingPadding))
    ));
}

#[test]
fn rejects_a_lone_base64_character() {
    assert!(matches!(
        decode(r#"{"audioContent": "Q"}"#),
        Err(VertexSpeechError::Base64(Base64Error::SingleCharacter))
    ));
}

#[test]
fn rejects_malformed_json() {
    assert!(matches!(
        decode("{nope"),
        Err(VertexSpeechError::ResponseJson(_))
    ));
}

#[rstest]
#[case::list("[]")]
#[case::text(r#""text""#)]
fn rejects_a_non_object_response(#[case] body: &str) {
    assert!(matches!(
        decode(body),
        Err(VertexSpeechError::ResponseNotObject)
    ));
}

#[test]
fn rejects_a_response_over_the_limit() {
    let body = vec![b' '; MAX_RESPONSE_BYTES.saturating_add(1)];
    assert!(matches!(
        decode_vertex_speech_audio(&body, &sniff_x),
        Err(VertexSpeechError::ResponseTooLarge { .. })
    ));
}

#[rstest]
fn replays_the_vertex_speech_audio_corpus(
    #[files("fuzz/corpus/vertex_speech_audio/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(decode_vertex_speech_audio(&bytes, &sniff_x));
}
