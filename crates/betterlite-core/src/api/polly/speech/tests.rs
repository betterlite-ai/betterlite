#![cfg(test)]

use std::time::{Duration, SystemTime};

use betterlite_models::decisions::Json;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::json;

use super::body::{is_ssml, request_body, serialize};
use super::config::PollySpeechConfig;
use super::credentials::{self, CredentialResolver, CredentialSources, ResolvedCredentials};
use super::endpoint::{DEFAULT_REGION, complete_url};
use super::error::SpeechError;
use super::mapping::map_openai_params;
use super::region::{FALLBACK_REGION, RegionEnvironment, dispatch_region};
use super::request::{
    Dispatch, SignedRequest, TextToSpeechRequest, configured_url, dispatch, transform_request,
};
use crate::api::speech_call::operations::{SpeechConfigOperation, SpeechMappingOptions};

const EXAMPLE_KEY: &str = "AKIDEXAMPLE";
const EXAMPLE_SECRET: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
const SIMPLE_BODY: &str = r#"{"Engine": "standard", "OutputFormat": "ogg_vorbis", "Text": "hello", "VoiceId": "Matthew", "TextType": "text"}"#;
const UNICODE_TEXT: &str = "héllo 世界 😀 <speak>hi</speak>";
const UNICODE_BODY: &str = concat!(
    r#"{"Engine": "neural", "OutputFormat": "mp3", "Text": "h"#,
    "\\u00e9llo \\u4e16\\u754c \\ud83d\\ude00",
    r#" <speak>hi</speak>", "VoiceId": "Joanna", "TextType": "ssml", "LanguageCode": null, "LexiconNames": ["a", "b"], "SampleRate": "22050"}"#,
);

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

fn voice_of(text: &str) -> Option<SpeechVoice> {
    serde_json::from_str(text).expect("a speech voice")
}

fn environment<'text>(
    region_name: Option<&'text str>,
    region: Option<&'text str>,
) -> RegionEnvironment<'text> {
    RegionEnvironment {
        region_name,
        region,
    }
}

fn fixed_now() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_767_225_600))
        .expect("the fixed signing time")
}

fn mapped(
    model: &str,
    voice: Option<&SpeechVoice>,
    optional_params: &str,
    kwargs: &str,
) -> (Option<SpeechVoice>, Object) {
    let params = object(optional_params);
    let forwarded = object(kwargs);
    let options = SpeechMappingOptions {
        voice: voice.cloned(),
        drop_params: false,
        kwargs: forwarded,
    };
    map_openai_params(model, &params, &options)
}

fn region_params(region: &str, api_base: Option<&str>) -> Object {
    let mut params = Object::new();
    params.insert(String::from("aws_region_name"), json!(region));
    if let Some(base) = api_base {
        params.insert(String::from("api_base"), json!(base));
    }
    params
}

struct FixedResolver {
    with_token: bool,
}

impl CredentialResolver for FixedResolver {
    fn resolve(
        &self,
        _sources: &CredentialSources<'_>,
    ) -> Result<ResolvedCredentials, SpeechError> {
        Ok(ResolvedCredentials {
            access_key_id: SecretString::from(EXAMPLE_KEY.to_owned()),
            secret_access_key: SecretString::from(EXAMPLE_SECRET.to_owned()),
            session_token: self
                .with_token
                .then_some(SecretString::from("session-token".to_owned())),
        })
    }
}

struct ExpectingResolver;

impl CredentialResolver for ExpectingResolver {
    fn resolve(&self, sources: &CredentialSources<'_>) -> Result<ResolvedCredentials, SpeechError> {
        assert_eq!(sources.aws_region_name, Some("eu-west-1"));
        assert_eq!(sources.aws_session_name, Some("session"));
        assert_eq!(sources.aws_access_key_id, Some(EXAMPLE_KEY));
        Ok(ResolvedCredentials {
            access_key_id: SecretString::from(EXAMPLE_KEY.to_owned()),
            secret_access_key: SecretString::from(EXAMPLE_SECRET.to_owned()),
            session_token: Some(SecretString::from("session-token".to_owned())),
        })
    }
}

fn transform(
    text: &str,
    voice: Option<&str>,
    optional_params: &Object,
    litellm_params: &Object,
    resolver: &dyn CredentialResolver,
) -> TextToSpeechRequest {
    transform_request(
        &SignedRequest {
            input: text,
            voice,
            optional_params,
            litellm_params,
            now: fixed_now(),
        },
        resolver,
    )
    .expect("a signed request")
}

struct SimpleVector {
    region: &'static str,
    api_base: Option<&'static str>,
    with_token: bool,
    url: &'static str,
    authorization: &'static str,
}

fn check_simple(vector: &SimpleVector) {
    let params = object(r#"{"engine": "standard", "output_format": "ogg_vorbis"}"#);
    let litellm_params = region_params(vector.region, vector.api_base);
    let resolver = FixedResolver {
        with_token: vector.with_token,
    };
    let signed = transform(
        "hello",
        Some("Matthew"),
        &params,
        &litellm_params,
        &resolver,
    );
    assert_eq!(signed.url, vector.url);
    assert_eq!(signed.data.ssml_body.as_deref(), Some(SIMPLE_BODY));
    let headers = signed.data.headers.expect("signed headers");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some(vector.authorization)
    );
    assert_eq!(
        headers.contains_key("X-Amz-Security-Token"),
        vector.with_token
    );
}

// the region rules of dispatch_text_to_speech, with the resolver's own precedence behind them

#[test]
fn the_kwargs_region_wins_over_the_optional_region_and_the_environment() {
    let resolved = dispatch_region(
        Some("eu-west-1"),
        Some("ap-south-1"),
        environment(Some("ca-central-1"), Some("sa-east-1")),
    );

    assert_eq!(resolved.unwrap(), "eu-west-1");
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
fn the_optional_region_is_used_when_the_kwargs_region_is_missing_or_empty(
    #[case] kwargs_region: Option<&str>,
) {
    let resolved = dispatch_region(
        kwargs_region,
        Some("ap-south-1"),
        environment(Some("ca-central-1"), Some("sa-east-1")),
    );

    assert_eq!(resolved.unwrap(), "ap-south-1");
}

#[rstest]
#[case::aws_region_wins(Some("eu-central-1"), Some("ca-central-1"), "eu-central-1")]
#[case::region_name_only(None, Some("ca-central-1"), "ca-central-1")]
#[case::builtin_fallback(None, None, FALLBACK_REGION)]
fn a_missing_optional_region_reads_the_environment(
    #[case] region_name: Option<&str>,
    #[case] region: Option<&str>,
    #[case] expected: &str,
) {
    let resolved = dispatch_region(None, None, environment(region_name, region));

    assert_eq!(resolved.unwrap(), expected);
}

#[test]
fn an_empty_optional_region_is_used_as_given() {
    let resolved = dispatch_region(None, Some(""), environment(None, None));

    assert_eq!(resolved.unwrap(), "");
}

#[rstest]
#[case::uppercase("US-EAST-1")]
#[case::underscore("us_east_1")]
#[case::empty("")]
#[case::space("us east 1")]
fn an_invalid_environment_region_is_rejected_with_the_litellm_message(#[case] name: &str) {
    let error = dispatch_region(None, None, environment(None, Some(name))).unwrap_err();

    assert_eq!(
        error.to_string(),
        format!(
            "Invalid AWS region format: '{name}'. Region names must contain only lowercase letters, digits, and hyphens."
        )
    );
}

#[test]
fn a_kwargs_region_is_used_without_validation() {
    let resolved = dispatch_region(Some("US-EAST-1"), None, environment(None, None));

    assert_eq!(resolved.unwrap(), "US-EAST-1");
}

// dispatch_text_to_speech over the effective parameters and the voice

#[test]
fn dispatch_replaces_the_region_base_and_key_in_the_effective_params() {
    let kwargs = object(r#"{"aws_region_name": "eu-west-1"}"#);
    let voice = voice_of("\"alloy\"");
    let optional_params = object(r#"{"engine": "neural", "output_version": "v1"}"#);
    let dispatched = dispatch(Dispatch {
        voice: voice.as_ref(),
        optional_params: optional_params.clone(),
        litellm_params: object(r#"{"aws_access_key_id": "AKIDEXAMPLE", "custom": 1}"#),
        kwargs: &kwargs,
        api_base: Some("https://example.test"),
        api_key: Some("sk-test"),
        environment: environment(Some("ca-central-1"), Some("sa-east-1")),
    })
    .unwrap();

    assert_eq!(dispatched.voice.as_deref(), Some("Joanna"));
    assert_eq!(
        dispatched
            .litellm_params
            .get("aws_region_name")
            .and_then(|value| value.as_str()),
        Some("eu-west-1")
    );
    assert_eq!(
        dispatched
            .litellm_params
            .get("api_base")
            .and_then(|value| value.as_str()),
        Some("https://example.test")
    );
    assert_eq!(
        dispatched
            .litellm_params
            .get("api_key")
            .and_then(|value| value.as_str()),
        Some("sk-test")
    );
    assert_eq!(
        dispatched
            .litellm_params
            .get("aws_access_key_id")
            .and_then(|value| value.as_str()),
        Some("AKIDEXAMPLE")
    );
    assert_eq!(dispatched.optional_params, optional_params);
}

#[test]
fn dispatch_writes_null_base_and_key_when_they_are_missing() {
    let kwargs = Object::new();
    let dispatched = dispatch(Dispatch {
        voice: None,
        optional_params: Object::new(),
        litellm_params: Object::new(),
        kwargs: &kwargs,
        api_base: None,
        api_key: None,
        environment: environment(None, None),
    })
    .unwrap();

    assert!(dispatched.litellm_params["api_base"].is_null());
    assert!(dispatched.litellm_params["api_key"].is_null());
    assert_eq!(
        dispatched
            .litellm_params
            .get("aws_region_name")
            .and_then(|value| value.as_str()),
        Some(FALLBACK_REGION)
    );
}

#[test]
fn dispatch_uses_the_optional_region_before_the_environment() {
    let kwargs = Object::new();
    let dispatched = dispatch(Dispatch {
        voice: None,
        optional_params: object(r#"{"aws_region_name": "ap-south-1"}"#),
        litellm_params: Object::new(),
        kwargs: &kwargs,
        api_base: None,
        api_key: None,
        environment: environment(Some("ca-central-1"), Some("sa-east-1")),
    })
    .unwrap();

    assert_eq!(
        dispatched
            .litellm_params
            .get("aws_region_name")
            .and_then(|value| value.as_str()),
        Some("ap-south-1")
    );
}

#[test]
fn dispatch_rejects_a_non_string_region_with_the_field_name() {
    let kwargs = Object::new();
    let error = dispatch(Dispatch {
        voice: None,
        optional_params: object(r#"{"aws_region_name": 3}"#),
        litellm_params: Object::new(),
        kwargs: &kwargs,
        api_base: None,
        api_key: None,
        environment: environment(None, None),
    })
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "the AWS Polly parameter aws_region_name must be a string"
    );
}

#[rstest]
#[case::mapping(r#"{"name": "Amy"}"#, Some("Amy"))]
#[case::mapping_without_name(r#"{"other": 1}"#, None)]
#[case::explicit_null("null", None)]
#[case::string("\"Matthew\"", Some("Matthew"))]
#[case::empty_string("\"\"", Some(""))]
fn dispatch_turns_a_voice_mapping_into_its_name_member(
    #[case] voice: &str,
    #[case] expected: Option<&str>,
) {
    let parsed = voice_of(voice);
    let kwargs = Object::new();
    let dispatched = dispatch(Dispatch {
        voice: parsed.as_ref(),
        optional_params: Object::new(),
        litellm_params: Object::new(),
        kwargs: &kwargs,
        api_base: None,
        api_key: None,
        environment: environment(None, None),
    })
    .unwrap();

    assert_eq!(dispatched.voice.as_deref(), expected);
}

// map_openai_params: voices, formats, engines and the Polly kwargs

#[rstest]
#[case::alloy("alloy", "Joanna")]
#[case::echo("echo", "Matthew")]
#[case::fable("fable", "Amy")]
#[case::onyx("onyx", "Brian")]
#[case::nova("nova", "Ivy")]
#[case::shimmer("shimmer", "Kendra")]
fn the_six_openai_voice_names_map_to_polly_voices(#[case] name: &str, #[case] expected: &str) {
    let voice = voice_of(&format!("\"{name}\""));
    let (mapped_voice, _) = mapped("aws_polly/neural", voice.as_ref(), "{}", "{}");

    assert_eq!(
        mapped_voice,
        Some(SpeechVoice::Name(String::from(expected)))
    );
}

#[rstest]
#[case::uppercase("ALLOY")]
#[case::polly_name("Joanna")]
#[case::spaced(" alloy")]
#[case::empty("")]
fn other_voice_names_keep_their_exact_case(#[case] name: &str) {
    let voice = voice_of(&format!("\"{name}\""));
    let (mapped_voice, _) = mapped("aws_polly/neural", voice.as_ref(), "{}", "{}");

    assert_eq!(mapped_voice, Some(SpeechVoice::Name(String::from(name))));
}

#[rstest]
#[case::mapping(r#"{"name": "Amy"}"#)]
#[case::explicit_null("null")]
fn a_non_string_voice_maps_to_no_voice(#[case] voice: &str) {
    let parsed = voice_of(voice);
    let (mapped_voice, _) = mapped("aws_polly/neural", parsed.as_ref(), "{}", "{}");

    assert_eq!(mapped_voice, None);
}

#[rstest]
#[case::mp3("mp3", "mp3")]
#[case::opus("opus", "ogg_vorbis")]
#[case::aac("aac", "mp3")]
#[case::flac("flac", "mp3")]
#[case::wav("wav", "pcm")]
#[case::pcm("pcm", "pcm")]
#[case::unknown("ogg", "ogg")]
#[case::empty("", "")]
fn response_formats_follow_the_polly_mappings(#[case] requested: &str, #[case] expected: &str) {
    let (_, params) = mapped(
        "aws_polly/neural",
        None,
        &format!(r#"{{"response_format": "{requested}"}}"#),
        "{}",
    );

    assert_eq!(
        params.get("output_format").and_then(|value| value.as_str()),
        Some(expected)
    );
}

#[test]
fn a_missing_or_null_response_format_defaults_to_mp3() {
    let (_, absent) = mapped("aws_polly/neural", None, "{}", "{}");
    let (_, nulled) = mapped(
        "aws_polly/neural",
        None,
        r#"{"response_format": null}"#,
        "{}",
    );

    assert_eq!(
        absent.get("output_format").and_then(|value| value.as_str()),
        Some("mp3")
    );
    assert_eq!(
        nulled.get("output_format").and_then(|value| value.as_str()),
        Some("mp3")
    );
}

#[rstest]
#[case::bare_standard("standard", "neural")]
#[case::neural("aws_polly/neural", "neural")]
#[case::standard("aws_polly/standard", "standard")]
#[case::long_form("aws_polly/long-form", "long-form")]
#[case::generative("aws_polly/generative", "generative")]
#[case::uppercase("aws_polly/Neural", "neural")]
#[case::bogus("aws_polly/bogus", "neural")]
#[case::no_slash("aws_polly", "neural")]
#[case::empty("", "neural")]
#[case::other_prefix("other/standard", "standard")]
#[case::leading_slash("/standard", "standard")]
#[case::third_component("aws_polly/standard/neural", "standard")]
fn the_engine_comes_from_the_second_slash_component(#[case] model: &str, #[case] expected: &str) {
    let (_, params) = mapped(model, None, "{}", "{}");

    assert_eq!(
        params.get("engine").and_then(|value| value.as_str()),
        Some(expected)
    );
}

#[test]
fn the_mapping_writes_the_format_and_the_engine_only() {
    let (voice, params) = mapped("aws_polly/neural", None, "{}", "{}");

    assert_eq!(voice, None);
    assert_eq!(params.len(), 2);
    assert!(params.contains_key("engine"));
    assert!(params.contains_key("output_format"));
}

#[test]
fn polly_kwargs_are_copied_under_their_aws_names_including_nulls() {
    let (_, params) = mapped(
        "aws_polly/neural",
        None,
        "{}",
        r#"{"language_code": "en-US", "lexicon_names": ["a", "b"], "sample_rate": "22050", "speed": 1.5, "extra": 1}"#,
    );

    assert_eq!(
        params.get("LanguageCode").and_then(|value| value.as_str()),
        Some("en-US")
    );
    assert_eq!(params.get("LexiconNames"), Some(&json!(["a", "b"])));
    assert_eq!(
        params.get("SampleRate").and_then(|value| value.as_str()),
        Some("22050")
    );
    assert!(!params.contains_key("speed"));
    assert!(!params.contains_key("Speed"));
    assert!(!params.contains_key("extra"));
}

#[test]
fn a_null_polly_kwarg_reaches_the_mapping_as_a_null() {
    let (_, params) = mapped("aws_polly/neural", None, "{}", r#"{"language_code": null}"#);

    assert!(params["LanguageCode"].is_null());
}

#[test]
fn a_missing_polly_kwarg_stays_out_of_the_mapping() {
    let (_, params) = mapped("aws_polly/neural", None, "{}", "{}");

    assert!(!params.contains_key("LanguageCode"));
    assert!(!params.contains_key("LexiconNames"));
    assert!(!params.contains_key("SampleRate"));
}

#[test]
fn mapping_leaves_the_given_params_and_kwargs_untouched() {
    let params = object(r#"{"response_format": "wav"}"#);
    let kwargs = object(r#"{"language_code": "en-US", "speed": 2}"#);
    let options = SpeechMappingOptions {
        voice: None,
        drop_params: false,
        kwargs: kwargs.clone(),
    };
    let _mapped = map_openai_params("aws_polly/standard", &params, &options);

    assert_eq!(params, object(r#"{"response_format": "wav"}"#));
    assert_eq!(options.kwargs, kwargs);
}

// the complete URL: an explicit base, else the partition host

#[rstest]
#[case::plain("https://example.test", "https://example.test/v1/speech")]
#[case::trailing("https://example.test/", "https://example.test/v1/speech")]
#[case::many_trailing("https://example.test///", "https://example.test/v1/speech")]
#[case::empty_base("", "/v1/speech")]
#[case::with_path("https://example.test/base", "https://example.test/base/v1/speech")]
fn an_explicit_base_gains_the_speech_path_after_trailing_slashes(
    #[case] base: &str,
    #[case] expected: &str,
) {
    assert_eq!(complete_url(Some(base), Some("eu-west-1")), expected);
}

#[rstest]
#[case::commercial("us-east-1", "https://polly.us-east-1.amazonaws.com/v1/speech")]
#[case::china("cn-north-1", "https://polly.cn-north-1.amazonaws.com.cn/v1/speech")]
#[case::gov("us-gov-west-1", "https://polly.us-gov-west-1.amazonaws.com/v1/speech")]
#[case::iso("us-iso-east-1", "https://polly.us-iso-east-1.c2s.ic.gov/v1/speech")]
#[case::iso_b(
    "us-isob-east-1",
    "https://polly.us-isob-east-1.sc2s.sgov.gov/v1/speech"
)]
#[case::iso_f(
    "us-isof-south-1",
    "https://polly.us-isof-south-1.csp.hci.ic.gov/v1/speech"
)]
#[case::iso_e(
    "eu-isoe-west-1",
    "https://polly.eu-isoe-west-1.cloud.adc-e.uk/v1/speech"
)]
fn the_partition_host_is_built_from_the_region(#[case] region: &str, #[case] expected: &str) {
    assert_eq!(complete_url(None, Some(region)), expected);
}

#[test]
fn the_partition_host_defaults_to_us_east_1_without_a_region() {
    assert_eq!(
        complete_url(None, None),
        format!("https://polly.{DEFAULT_REGION}.amazonaws.com/v1/speech")
    );
}

#[test]
fn an_empty_region_is_used_as_given_in_the_partition_host() {
    assert_eq!(
        complete_url(None, Some("")),
        "https://polly..amazonaws.com/v1/speech"
    );
}

#[test]
fn configured_url_reads_the_base_and_the_region_from_the_effective_params() {
    let based = object(r#"{"api_base": "https://example.test/", "aws_region_name": "cn-north-1"}"#);
    let region_only = object(r#"{"aws_region_name": "cn-north-1"}"#);
    let null_base = object(r#"{"api_base": null, "aws_region_name": "us-west-2"}"#);

    assert_eq!(
        configured_url(&based).unwrap(),
        "https://example.test/v1/speech"
    );
    assert_eq!(
        configured_url(&region_only).unwrap(),
        "https://polly.cn-north-1.amazonaws.com.cn/v1/speech"
    );
    assert_eq!(
        configured_url(&null_base).unwrap(),
        "https://polly.us-west-2.amazonaws.com/v1/speech"
    );
}

// the synthesis body and its Python-compatible JSON

#[rstest]
#[case::tag("<speak>hi</speak>", true)]
#[case::attributes("<speak xml:lang=\"en-US\">hi</speak>", true)]
#[case::dangling_prefix("<speak ", true)]
#[case::inside("hello <speak>there", true)]
#[case::uppercase("<SPEAK>hi</SPEAK>", false)]
#[case::other_tag("<speakx>hi</speakx>", false)]
#[case::incomplete("<speak", false)]
#[case::empty("", false)]
fn ssml_is_detected_only_by_the_exact_lowercase_tags(#[case] text: &str, #[case] expected: bool) {
    assert_eq!(is_ssml(text), expected);
}

#[test]
fn the_body_fields_keep_their_litellm_order_and_values() {
    let params = object(
        r#"{"engine": "standard", "output_format": "ogg_vorbis", "SampleRate": "22050", "LanguageCode": null}"#,
    );
    let body = request_body("hello", Some("Matthew"), &params);

    assert_eq!(
        body.keys().map(String::as_str).collect::<Vec<_>>(),
        vec![
            "Engine",
            "OutputFormat",
            "Text",
            "VoiceId",
            "TextType",
            "LanguageCode",
            "SampleRate",
        ]
    );
    assert_eq!(body["Engine"], Json::Text(String::from("standard")));
    assert_eq!(body["OutputFormat"], Json::Text(String::from("ogg_vorbis")));
    assert_eq!(body["Text"], Json::Text(String::from("hello")));
    assert_eq!(body["VoiceId"], Json::Text(String::from("Matthew")));
    assert_eq!(body["TextType"], Json::Text(String::from("text")));
    assert_eq!(body["LanguageCode"], Json::Null);
    assert_eq!(body["SampleRate"], Json::Text(String::from("22050")));
}

#[test]
fn an_ssml_text_sets_the_ssml_text_type() {
    let body = request_body("<speak>hi</speak>", None, &Object::new());

    assert_eq!(body["TextType"], Json::Text(String::from("ssml")));
    assert_eq!(body["VoiceId"], Json::Text(String::from("Joanna")));
}

#[test]
fn the_body_defaults_its_engine_and_format() {
    let nulled = object(r#"{"engine": null, "output_format": null}"#);
    let absent = request_body("hi", None, &Object::new());
    let null_body = request_body("hi", None, &nulled);

    assert_eq!(absent["Engine"], Json::Text(String::from("neural")));
    assert_eq!(absent["OutputFormat"], Json::Text(String::from("mp3")));
    assert_eq!(null_body["Engine"], Json::Text(String::from("neural")));
    assert_eq!(null_body["OutputFormat"], Json::Text(String::from("mp3")));
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
fn the_request_voice_is_joanna_for_a_missing_or_empty_voice(#[case] voice: Option<&str>) {
    let body = request_body("hi", voice, &Object::new());

    assert_eq!(body["VoiceId"], Json::Text(String::from("Joanna")));
}

#[test]
fn a_named_voice_reaches_the_body() {
    let body = request_body("hi", Some("Matthew"), &Object::new());

    assert_eq!(body["VoiceId"], Json::Text(String::from("Matthew")));
}

#[test]
fn the_body_serializes_like_pythons_json_dumps() {
    let params = object(
        r#"{"engine": "neural", "output_format": "mp3", "LanguageCode": null, "LexiconNames": ["a", "b"], "SampleRate": "22050"}"#,
    );
    let body = request_body(UNICODE_TEXT, Some("Joanna"), &params);

    assert_eq!(serialize(&body), UNICODE_BODY);
}

// the signed request: exact botocore vectors, service polly, the body bytes carried in ssml_body

#[test]
fn the_signed_request_is_the_botocore_unicode_payload() {
    let params = object(
        r#"{"engine": "neural", "output_format": "mp3", "LanguageCode": null, "LexiconNames": ["a", "b"], "SampleRate": "22050"}"#,
    );
    let litellm_params = object(r#"{"aws_region_name": "us-east-1"}"#);
    let signed = transform(
        UNICODE_TEXT,
        Some("Joanna"),
        &params,
        &litellm_params,
        &FixedResolver { with_token: true },
    );

    assert_eq!(
        signed.url,
        "https://polly.us-east-1.amazonaws.com/v1/speech"
    );
    assert_eq!(signed.data.ssml_body.as_deref(), Some(UNICODE_BODY));
    assert!(signed.data.dict_body.is_none());
    let headers = signed.data.headers.expect("signed headers");
    assert_eq!(headers.len(), 4);
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        headers.get("X-Amz-Date").map(String::as_str),
        Some("20260101T000000Z")
    );
    assert_eq!(
        headers.get("X-Amz-Security-Token").map(String::as_str),
        Some("session-token")
    );
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some(
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-east-1/polly/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token, Signature=2d4c4775e7eb71028f3e027d3caf3930e25efdc46d524d5f0d8ca1961cbb7022"
        )
    );
}

#[test]
fn the_simple_payload_is_signed_for_us_west_2_without_a_token() {
    check_simple(&SimpleVector {
        region: "us-west-2",
        api_base: None,
        with_token: false,
        url: "https://polly.us-west-2.amazonaws.com/v1/speech",
        authorization: "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-west-2/polly/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=8155fdb35d8c7a1b8e4a69d63279c092e32525d65f0ced9185fcfb130ffa5eb9",
    });
}

#[test]
fn the_partition_host_is_signed_for_cn_north_1() {
    check_simple(&SimpleVector {
        region: "cn-north-1",
        api_base: None,
        with_token: true,
        url: "https://polly.cn-north-1.amazonaws.com.cn/v1/speech",
        authorization: "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/cn-north-1/polly/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token, Signature=b371695ff9c8e21fb2dc584e7b84d218f51a0642b8cc9b20bc03f89f3fa10f50",
    });
}

#[test]
fn an_explicit_base_is_signed_after_the_speech_path_is_appended() {
    check_simple(&SimpleVector {
        region: "eu-west-1",
        api_base: Some("https://example.test"),
        with_token: true,
        url: "https://example.test/v1/speech",
        authorization: "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/eu-west-1/polly/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token, Signature=84a5fc1799e4989d1b7f61cfbf5c3f895e45bbed38f70050c7bf1fb327e0e773",
    });
}

#[test]
fn the_gov_partition_host_is_signed_for_us_gov_west_1() {
    check_simple(&SimpleVector {
        region: "us-gov-west-1",
        api_base: None,
        with_token: true,
        url: "https://polly.us-gov-west-1.amazonaws.com/v1/speech",
        authorization: "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-gov-west-1/polly/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-security-token, Signature=4338bf8d2ff2c3e5a162a0ed0c0d9a71b90cc6925f908512f2b788eff524ebee",
    });
}

#[test]
fn transform_resolves_credentials_from_the_effective_params() {
    let params = object(r#"{"engine": "neural", "output_format": "mp3"}"#);
    let litellm_params = object(
        r#"{"aws_region_name": "eu-west-1", "aws_session_name": "session", "aws_access_key_id": "AKIDEXAMPLE"}"#,
    );
    let signed = transform("hello", None, &params, &litellm_params, &ExpectingResolver);

    assert!(signed.data.ssml_body.is_some());
}

// the ten get_credentials arguments

#[test]
fn the_credential_sources_carry_all_ten_arguments() {
    let params = object(
        r#"{"aws_access_key_id": "AKID", "aws_secret_access_key": "SECRET", "aws_session_token": "TOKEN", "aws_region_name": "eu-west-1", "aws_session_name": "session", "aws_profile_name": "profile", "aws_role_name": "role", "aws_web_identity_token": "wid", "aws_sts_endpoint": "sts", "aws_external_id": "ext"}"#,
    );
    let sources = credentials::sources(&params).unwrap();

    assert_eq!(sources.aws_access_key_id, Some("AKID"));
    assert_eq!(sources.aws_secret_access_key, Some("SECRET"));
    assert_eq!(sources.aws_session_token, Some("TOKEN"));
    assert_eq!(sources.aws_region_name, Some("eu-west-1"));
    assert_eq!(sources.aws_session_name, Some("session"));
    assert_eq!(sources.aws_profile_name, Some("profile"));
    assert_eq!(sources.aws_role_name, Some("role"));
    assert_eq!(sources.aws_web_identity_token, Some("wid"));
    assert_eq!(sources.aws_sts_endpoint, Some("sts"));
    assert_eq!(sources.aws_external_id, Some("ext"));
}

#[test]
fn an_absent_region_reaches_the_resolver_as_us_east_1() {
    let params = Object::new();
    let sources = credentials::sources(&params).unwrap();

    assert_eq!(sources.aws_region_name, Some(DEFAULT_REGION));
    assert_eq!(sources.aws_access_key_id, None);
}

#[test]
fn a_null_region_reaches_the_resolver_as_null() {
    let params = object(r#"{"aws_region_name": null}"#);
    let sources = credentials::sources(&params).unwrap();

    assert_eq!(sources.aws_region_name, None);
}

#[test]
fn a_non_string_credential_is_rejected_with_the_field_name() {
    let error = credentials::sources(&object(r#"{"aws_access_key_id": 1}"#)).unwrap_err();

    assert_eq!(
        error.to_string(),
        "the AWS Polly parameter aws_access_key_id must be a string"
    );
}

#[test]
fn the_speech_config_operation_delegates_to_the_mapping() {
    let params = object(r#"{"response_format": "opus"}"#);
    let options = SpeechMappingOptions {
        voice: voice_of(r#""alloy""#),
        drop_params: false,
        kwargs: Object::new(),
    };
    let expected = map_openai_params("aws_polly/standard", &params, &options);
    let actual = PollySpeechConfig.map_openai_params("aws_polly/standard", params, &options);

    assert_eq!(actual, expected);
    assert_eq!(actual.1["engine"], json!("standard"));
}
