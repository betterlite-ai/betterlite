#![cfg(test)]

use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::auth::{SpeechKeys, validate_environment};
use super::endpoint::complete_url;
use super::error::SpeechError;
use super::mapping::{SpeechMapping, map_openai_params};
use super::request::transform_request;
use super::voice::resolve_voice_id;
use crate::api::elevenlabs::common::ElevenLabsError;

const ENVIRONMENT_BASE: &str = "https://api.elevenlabs.io";
const ALLOY_ID: &str = "21m00Tcm4TlvDq8ikWAM";
const ASH_ID: &str = concat!("AZnzlk1XvdvU", "eBnXmlld");

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

fn voice_of(text: &str) -> Option<SpeechVoice> {
    serde_json::from_str(text).expect("a speech voice")
}

struct Mapped {
    mapping: SpeechMapping,
    optional_params: Object,
    kwargs: Object,
}

fn map(voice: &str, optional_params: &str, kwargs: &str) -> Mapped {
    let mut optional_params = object(optional_params);
    let mut kwargs = object(kwargs);
    let mapping = map_openai_params(&mut optional_params, voice_of(voice).as_ref(), &mut kwargs)
        .expect("mapped speech params");
    Mapped {
        mapping,
        optional_params,
        kwargs,
    }
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

#[rstest]
#[case::alloy("alloy", "21m00Tcm4TlvDq8ikWAM")]
#[case::amber("amber", "5Q0t7uMcjvnagumLfvZi")]
#[case::ash("ash", ASH_ID)]
#[case::august("august", "D38z5RcWu1voky8WS1ja")]
#[case::blue("blue", "2EiwWnXFnvU5JabPnv8n")]
#[case::coral("coral", "9BWtsMINqrJLrRacOk9x")]
#[case::lily("lily", "EXAVITQu4vr4xnSDxMaL")]
#[case::onyx("onyx", "29vD33N1CtxCmqQRPOHJ")]
#[case::sage("sage", "CwhRBWXzGAHq8TQ4Fs17")]
#[case::verse("verse", "CYw3kZ02Hs0563khs1Fj")]
#[case::upper_case("ALLOY", "21m00Tcm4TlvDq8ikWAM")]
#[case::trimmed("  Sage  ", "CwhRBWXzGAHq8TQ4Fs17")]
#[case::unknown_keeps_its_case("  Custom-Voice ", "Custom-Voice")]
#[case::separators_are_stripped("\u{1f}alloy\u{1c}", ALLOY_ID)]
#[case::unicode_spaces_are_stripped("\u{a0}ash\u{3000}", ASH_ID)]
fn named_voices_resolve_through_the_case_insensitive_mappings(
    #[case] name: &str,
    #[case] expected: &str,
) {
    let mut params = Object::new();
    let resolved = resolve_voice_id(Some(&SpeechVoice::Name(name.to_owned())), &mut params)
        .expect("a resolved voice");

    assert_eq!(resolved, expected);
}

#[rstest]
#[case::voice_id_wins(
    r#"{"voice_id": "lily", "id": "ash", "name": "blue"}"#,
    "EXAVITQu4vr4xnSDxMaL"
)]
#[case::id_before_name(r#"{"id": "blue", "name": "coral"}"#, "2EiwWnXFnvU5JabPnv8n")]
#[case::blank_voice_id_falls_to_id(r#"{"voice_id": "  ", "id": "ash"}"#, ASH_ID)]
#[case::separator_voice_id_falls_to_id(r#"{"voice_id": "\u001c", "id": "ash"}"#, ASH_ID)]
#[case::non_string_keys_fall_to_name(
    r#"{"voice_id": 5, "id": null, "name": "Onyx"}"#,
    "29vD33N1CtxCmqQRPOHJ"
)]
#[case::unknown_name_is_trimmed(r#"{"name": " Rachel "}"#, "Rachel")]
fn mapping_voices_use_the_first_nonblank_string_key(#[case] voice: &str, #[case] expected: &str) {
    let mut params = Object::new();
    let resolved =
        resolve_voice_id(voice_of(voice).as_ref(), &mut params).expect("a resolved voice");

    assert_eq!(resolved, expected);
}

#[rstest]
#[case::empty("")]
#[case::spaces("   ")]
#[case::separators("\u{1c}\u{1f}")]
fn a_blank_voice_string_resolves_to_an_empty_id_without_consuming_voice_id(#[case] name: &str) {
    let mut params = object(r#"{"voice_id": "lily"}"#);
    let resolved = resolve_voice_id(Some(&SpeechVoice::Name(name.to_owned())), &mut params)
        .expect("a resolved voice");

    assert_eq!(resolved, "");
    assert!(params.contains_key("voice_id"));
}

#[rstest]
#[case::no_voice("null")]
#[case::blank_voice_id_key(r#"{"voice_id": " "}"#)]
#[case::unrelated_keys(r#"{"other": 1}"#)]
fn an_unresolved_voice_takes_and_removes_the_optional_voice_id(#[case] voice: &str) {
    let mut params = object(r#"{"voice_id": " Lily ", "seed": 1}"#);
    let resolved =
        resolve_voice_id(voice_of(voice).as_ref(), &mut params).expect("a resolved voice");

    assert_eq!(resolved, "EXAVITQu4vr4xnSDxMaL");
    assert_eq!(params, object(r#"{"seed": 1}"#));
}

#[test]
fn a_resolved_voice_leaves_the_optional_voice_id_for_the_body() {
    let mapped = map(r#""alloy""#, r#"{"voice_id": "other"}"#, "{}");

    assert_eq!(mapped.mapping.voice_id, ALLOY_ID);
    assert_eq!(mapped.mapping.params, object(r#"{"voice_id": "other"}"#));
}

#[rstest]
#[case::missing("{}")]
#[case::blank(r#"{"voice_id": "  "}"#)]
#[case::separator(r#"{"voice_id": "\u001c"}"#)]
#[case::not_a_string(r#"{"voice_id": 7}"#)]
fn a_missing_voice_is_an_error(#[case] options: &str) {
    let mut params = object(options);
    let resolved = resolve_voice_id(None, &mut params);

    assert!(matches!(resolved, Err(SpeechError::VoiceRequired)));
}

#[rstest]
#[case::mp3(
    r#"{"response_format": "mp3"}"#,
    r#"{"output_format": "mp3_44100_128"}"#
)]
#[case::pcm(r#"{"response_format": "pcm"}"#, r#"{"output_format": "pcm_44100"}"#)]
#[case::opus(
    r#"{"response_format": "opus"}"#,
    r#"{"output_format": "opus_48000_128"}"#
)]
#[case::other_strings_pass_through(
    r#"{"response_format": "mp3_22050_32"}"#,
    r#"{"output_format": "mp3_22050_32"}"#
)]
#[case::wav_passes_through(r#"{"response_format": "wav"}"#, r#"{"output_format": "wav"}"#)]
fn response_format_becomes_the_output_format_query(#[case] options: &str, #[case] query: &str) {
    let mapped = map(r#""alloy""#, options, "{}");
    let expected = object(&format!(
        r#"{{"__elevenlabs_query_params__": {query}, "__elevenlabs_voice_id__": "{ALLOY_ID}"}}"#
    ));

    assert_eq!(mapped.kwargs, expected);
    assert!(mapped.mapping.params.is_empty());
}

#[rstest]
#[case::absent_is_never_defaulted("{}")]
#[case::null(r#"{"response_format": null}"#)]
#[case::not_a_string(r#"{"response_format": 3}"#)]
fn a_missing_or_invalid_response_format_adds_no_query(#[case] options: &str) {
    let mapped = map(r#""alloy""#, options, "{}");

    assert_eq!(
        mapped.kwargs,
        object(&format!(r#"{{"__elevenlabs_voice_id__": "{ALLOY_ID}"}}"#))
    );
    assert!(mapped.mapping.params.is_empty());
}

#[test]
fn an_empty_query_drops_a_stale_query_key_from_kwargs() {
    let mapped = map(
        r#""alloy""#,
        "{}",
        r#"{"__elevenlabs_query_params__": {"output_format": "mp3"}}"#,
    );

    assert_eq!(
        mapped.kwargs,
        object(&format!(r#"{{"__elevenlabs_voice_id__": "{ALLOY_ID}"}}"#))
    );
    assert!(mapped.mapping.params.is_empty());
}

#[test]
fn a_new_query_replaces_a_stale_query_key_in_kwargs() {
    let mapped = map(
        r#""alloy""#,
        r#"{"response_format": "opus"}"#,
        r#"{"__elevenlabs_query_params__": {"output_format": "mp3"}}"#,
    );

    assert_eq!(
        mapped.kwargs,
        object(&format!(
            r#"{{"__elevenlabs_query_params__": {{"output_format": "opus_48000_128"}}, "__elevenlabs_voice_id__": "{ALLOY_ID}"}}"#
        ))
    );
    assert!(mapped.mapping.params.is_empty());
}

#[rstest]
#[case::float("1.5", r#"{"voice_settings": {"speed": 1.5}}"#)]
#[case::integer("2", r#"{"voice_settings": {"speed": 2.0}}"#)]
#[case::numeric_string(r#"" 0.75 ""#, r#"{"voice_settings": {"speed": 0.75}}"#)]
#[case::separator_padded_string(r#""\u001f0.75\u001c""#, "{}")]
#[case::boolean("true", r#"{"voice_settings": {"speed": 1.0}}"#)]
#[case::text(r#""fast""#, "{}")]
#[case::list("[1]", "{}")]
#[case::object(r#"{"a": 1}"#, "{}")]
#[case::null("null", "{}")]
#[case::infinity(r#""inf""#, "{}")]
#[case::not_a_number(r#""nan""#, "{}")]
fn speed_is_parsed_into_voice_settings(#[case] speed: &str, #[case] expected: &str) {
    let mapped = map(r#""alloy""#, &format!(r#"{{"speed": {speed}}}"#), "{}");

    assert_eq!(mapped.mapping.params, object(expected));
}

#[test]
fn speed_joins_existing_voice_settings_and_mutates_the_callers_object() {
    let mapped = map(
        r#""alloy""#,
        r#"{"speed": 1.2, "voice_settings": {"stability": 0.4}}"#,
        "{}",
    );

    assert_eq!(
        mapped.mapping.params,
        object(r#"{"voice_settings": {"stability": 0.4, "speed": 1.2}}"#)
    );
    assert_eq!(
        mapped.optional_params,
        object(r#"{"speed": 1.2, "voice_settings": {"stability": 0.4, "speed": 1.2}}"#)
    );
}

#[test]
fn speed_replaces_voice_settings_that_are_not_an_object() {
    let mapped = map(
        r#""alloy""#,
        r#"{"speed": 1.2, "voice_settings": "calm"}"#,
        "{}",
    );

    assert_eq!(
        mapped.mapping.params,
        object(r#"{"voice_settings": {"speed": 1.2}}"#)
    );
    assert_eq!(
        mapped.optional_params,
        object(r#"{"speed": 1.2, "voice_settings": "calm"}"#)
    );
}

#[test]
fn an_invalid_speed_keeps_existing_voice_settings_untouched() {
    let mapped = map(
        r#""alloy""#,
        r#"{"speed": "fast", "voice_settings": {"stability": 0.4}}"#,
        "{}",
    );

    assert_eq!(
        mapped.mapping.params,
        object(r#"{"voice_settings": {"stability": 0.4}}"#)
    );
    assert_eq!(
        mapped.optional_params,
        object(r#"{"speed": "fast", "voice_settings": {"stability": 0.4}}"#)
    );
}

#[test]
fn instructions_and_null_options_are_dropped_from_the_mapped_params() {
    let mapped = map(
        r#""alloy""#,
        r#"{"instructions": "whisper", "stability": null, "seed": 3}"#,
        "{}",
    );

    assert_eq!(mapped.mapping.params, object(r#"{"seed": 3}"#));
    assert!(mapped.optional_params.contains_key("instructions"));
}

#[test]
fn kwargs_extra_body_is_merged_over_options_and_removed() {
    let mapped = map(
        r#""alloy""#,
        r#"{"seed": 1, "stability": 2}"#,
        r#"{"extra_body": {"seed": 3, "language_code": null, "style": 4}}"#,
    );

    assert_eq!(
        mapped.mapping.params,
        object(r#"{"seed": 3, "stability": 2, "style": 4}"#)
    );
    assert!(!mapped.kwargs.contains_key("extra_body"));
}

#[test]
fn a_non_object_kwargs_extra_body_is_dropped_without_merging() {
    let mapped = map(r#""alloy""#, "{}", r#"{"extra_body": "text"}"#);

    assert!(mapped.mapping.params.is_empty());
    assert!(!mapped.kwargs.contains_key("extra_body"));
}

#[test]
fn plain_kwargs_are_consumed_over_extra_body_and_options() {
    let mapped = map(
        r#""alloy""#,
        r#"{"seed": 1}"#,
        r#"{"extra_body": {"seed": 3}, "seed": 9, "language_code": "en"}"#,
    );

    assert_eq!(
        mapped.mapping.params,
        object(r#"{"seed": 9, "language_code": "en"}"#)
    );
    assert_eq!(
        mapped.kwargs,
        object(&format!(r#"{{"__elevenlabs_voice_id__": "{ALLOY_ID}"}}"#))
    );
}

#[rstest]
#[case::voice("voice")]
#[case::model("model")]
#[case::response_format("response_format")]
#[case::output_format("output_format")]
#[case::user("user")]
#[case::litellm_owned_api_key("api_key")]
#[case::litellm_owned_metadata("metadata")]
fn reserved_and_litellm_owned_kwargs_stay_in_kwargs(#[case] name: &str) {
    let mapped = map(r#""alloy""#, "{}", &format!(r#"{{"{name}": "kept"}}"#));

    assert_eq!(
        mapped.kwargs,
        object(&format!(
            r#"{{"{name}": "kept", "__elevenlabs_voice_id__": "{ALLOY_ID}"}}"#
        ))
    );
    assert!(mapped.mapping.params.is_empty());
}

#[test]
fn a_null_kwarg_stays_in_kwargs_and_the_voice_key_is_overwritten() {
    let mapped = map(
        r#""alloy""#,
        "{}",
        r#"{"seed": null, "__elevenlabs_voice_id__": "stale"}"#,
    );

    assert_eq!(
        mapped.kwargs,
        object(&format!(
            r#"{{"seed": null, "__elevenlabs_voice_id__": "{ALLOY_ID}"}}"#
        ))
    );
    assert!(mapped.mapping.params.is_empty());
}

#[test]
fn the_callers_options_are_not_mutated_by_the_mapping() {
    let options = r#"{"voice_id": "lily", "response_format": "mp3", "instructions": "x"}"#;
    let mapped = map("null", options, "{}");

    assert_eq!(mapped.optional_params, object(options));
}

#[test]
fn the_request_body_starts_with_text_and_model_then_overlays_options() {
    let request = transform_request(
        "eleven_multilingual_v2",
        "hello",
        &object(r#"{"seed": 1, "stability": null}"#),
    );

    assert_eq!(
        request.dict_body,
        Some(object(
            r#"{"text": "hello", "model_id": "eleven_multilingual_v2", "seed": 1}"#
        ))
    );
    assert_eq!(request.ssml_body, None);
}

#[rstest]
#[case::options_override(
    r#"{"text": "from options", "model_id": "from_options"}"#,
    r#"{"text": "from options", "model_id": "from_options"}"#
)]
#[case::extra_body_overrides_options(
    r#"{"text": "from options", "extra_body": {"text": "from extra", "model_id": "from_extra"}}"#,
    r#"{"text": "from extra", "model_id": "from_extra"}"#
)]
#[case::null_extra_body_members_are_ignored(
    r#"{"extra_body": {"text": null, "seed": 5}}"#,
    r#"{"text": "hello", "model_id": "model", "seed": 5}"#
)]
#[case::non_object_extra_body_is_ignored(
    r#"{"extra_body": "text"}"#,
    r#"{"text": "hello", "model_id": "model"}"#
)]
fn request_body_overrides_follow_the_overlay_order(#[case] options: &str, #[case] expected: &str) {
    let request = transform_request("model", "hello", &object(options));

    assert_eq!(request.dict_body, Some(object(expected)));
}

#[test]
fn the_request_headers_fragment_has_only_the_content_type() {
    let request = transform_request("model", "hello", &Object::new());
    let mut expected = IndexMap::new();
    expected.insert("Content-Type".to_owned(), "application/json".to_owned());

    assert_eq!(request.headers, Some(expected));
}

#[test]
fn transforming_the_request_leaves_the_options_untouched() {
    let options = object(r#"{"extra_body": {"seed": 1}, "stability": 2}"#);
    let before = options.clone();
    let request = transform_request("model", "hello", &options);

    assert_eq!(options, before);
    assert!(
        request
            .dict_body
            .is_some_and(|body| !body.contains_key("extra_body"))
    );
}

fn url_params(voice_id: &str, query: Option<&str>) -> Object {
    let suffix = query.map_or_else(String::new, |text| {
        format!(r#", "__elevenlabs_query_params__": {text}"#)
    });
    object(&format!(
        r#"{{"__elevenlabs_voice_id__": {voice_id}{suffix}}}"#
    ))
}

#[rstest]
#[case::default_base(None, "https://api.elevenlabs.io/v1/text-to-speech/voice")]
#[case::explicit_base_wins(
    Some("https://proxy.test"),
    "https://proxy.test/v1/text-to-speech/voice"
)]
#[case::empty_base_uses_the_environment(
    Some(""),
    "https://api.elevenlabs.io/v1/text-to-speech/voice"
)]
#[case::trailing_slashes_are_stripped(
    Some("https://proxy.test//"),
    "https://proxy.test/v1/text-to-speech/voice"
)]
fn the_url_base_prefers_the_explicit_value(#[case] api_base: Option<&str>, #[case] expected: &str) {
    let url = complete_url(api_base, ENVIRONMENT_BASE, &url_params(r#""voice""#, None))
        .expect("a speech URL");

    assert_eq!(url, expected);
}

#[test]
fn the_environment_base_loses_its_trailing_slash() {
    let url = complete_url(None, "https://env.test/", &url_params(r#""voice""#, None))
        .expect("a speech URL");

    assert_eq!(url, "https://env.test/v1/text-to-speech/voice");
}

#[rstest]
#[case::plain("21m00Tcm4TlvDq8ikWAM", "21m00Tcm4TlvDq8ikWAM")]
#[case::space("my voice", "my%20voice")]
#[case::slash("a/b", "a%2Fb")]
#[case::question_mark("a?b", "a%3Fb")]
#[case::unicode("vöice", "v%C3%B6ice")]
fn the_voice_id_is_one_encoded_path_segment(#[case] voice_id: &str, #[case] encoded: &str) {
    let url = complete_url(
        None,
        ENVIRONMENT_BASE,
        &url_params(&format!(r#""{voice_id}""#), None),
    )
    .expect("a speech URL");

    assert_eq!(
        url,
        format!("{ENVIRONMENT_BASE}/v1/text-to-speech/{encoded}")
    );
}

#[rstest]
#[case::dot(r#"".""#)]
#[case::dot_dot(r#""..""#)]
fn a_voice_id_that_is_a_dot_segment_cannot_build_a_url(#[case] voice_id: &str) {
    let result = complete_url(None, ENVIRONMENT_BASE, &url_params(voice_id, None));

    assert!(matches!(result, Err(SpeechError::VoiceSegment { .. })));
}

#[rstest]
#[case::empty(r#""""#)]
#[case::spaces(r#""   ""#)]
#[case::separator(r#""\u001c""#)]
#[case::not_a_string("4")]
#[case::null("null")]
fn a_blank_or_missing_voice_id_cannot_build_a_url(#[case] voice_id: &str) {
    let result = complete_url(None, ENVIRONMENT_BASE, &url_params(voice_id, None));

    assert!(matches!(result, Err(SpeechError::VoiceRequired)));
}

#[test]
fn a_missing_voice_id_key_cannot_build_a_url() {
    let result = complete_url(None, ENVIRONMENT_BASE, &Object::new());

    assert!(matches!(result, Err(SpeechError::VoiceRequired)));
}

#[rstest]
#[case::spaces(r#""  ""#)]
#[case::separator(r#""\u001c""#)]
fn a_whitespace_voice_maps_to_an_empty_id_that_fails_url_building(#[case] voice: &str) {
    let mapped = map(voice, r#"{"voice_id": "lily"}"#, "{}");
    let result = complete_url(None, ENVIRONMENT_BASE, &mapped.kwargs);

    assert_eq!(mapped.mapping.voice_id, "");
    assert!(matches!(result, Err(SpeechError::VoiceRequired)));
}

#[rstest]
#[case::absent(None, "")]
#[case::null(Some("null"), "")]
#[case::empty_object(Some("{}"), "")]
#[case::output_format(
    Some(r#"{"output_format": "mp3_44100_128"}"#),
    "?output_format=mp3_44100_128"
)]
#[case::space_becomes_plus(Some(r#"{"q": "a b"}"#), "?q=a+b")]
#[case::star_is_encoded(Some(r#"{"q": "a*b"}"#), "?q=a%2Ab")]
#[case::tilde_is_kept(Some(r#"{"q": "a~b"}"#), "?q=a~b")]
#[case::literal_percent_seven_e(Some(r#"{"q": "%7E"}"#), "?q=%257E")]
#[case::number_value(Some(r#"{"n": 5}"#), "?n=5")]
#[case::pairs_keep_their_order(Some(r#"{"a": "1", "b": "2"}"#), "?a=1&b=2")]
fn the_query_is_appended_with_urlencode_rules(#[case] query: Option<&str>, #[case] suffix: &str) {
    let url = complete_url(None, ENVIRONMENT_BASE, &url_params(r#""voice""#, query))
        .expect("a speech URL");

    assert_eq!(
        url,
        format!("{ENVIRONMENT_BASE}/v1/text-to-speech/voice{suffix}")
    );
}

#[rstest]
#[case::string(r#""output_format=mp3""#)]
#[case::number("1")]
#[case::boolean("true")]
#[case::list(r#"[["a", "b"]]"#)]
fn a_query_that_is_not_an_object_is_an_error(#[case] query: &str) {
    let result = complete_url(
        None,
        ENVIRONMENT_BASE,
        &url_params(r#""voice""#, Some(query)),
    );

    assert!(matches!(result, Err(SpeechError::QueryShape)));
}

#[rstest]
#[case::list(r#"{"a": ["x", "y"]}"#, "a")]
#[case::boolean(r#"{"ok": true}"#, "ok")]
#[case::null(r#"{"gap": null}"#, "gap")]
fn a_query_value_that_is_not_text_or_a_number_names_its_key(
    #[case] query: &str,
    #[case] name: &str,
) {
    let result = complete_url(
        None,
        ENVIRONMENT_BASE,
        &url_params(r#""voice""#, Some(query)),
    );

    assert!(matches!(
        result,
        Err(SpeechError::QueryValue { name: ref found }) if found == name
    ));
}

#[test]
fn a_mapped_voice_and_format_build_the_whole_url() {
    let mapped = map(r#""Lily""#, r#"{"response_format": "mp3"}"#, "{}");
    let url = complete_url(
        Some("https://proxy.test/"),
        ENVIRONMENT_BASE,
        &mapped.kwargs,
    )
    .expect("a speech URL");

    assert_eq!(
        url,
        "https://proxy.test/v1/text-to-speech/EXAVITQu4vr4xnSDxMaL?output_format=mp3_44100_128"
    );
}

fn headers(entries: &[(&str, &str)]) -> IndexMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::explicit_first(Some("a"), Some("b"), Some("c"), Some("d"), "a")]
#[case::global_key_second(None, Some("b"), Some("c"), Some("d"), "b")]
#[case::openai_key_third(None, None, Some("c"), Some("d"), "c")]
#[case::environment_last(None, None, None, Some("d"), "d")]
#[case::empty_explicit_falls_through(Some(""), Some("b"), None, None, "b")]
#[case::empty_global_falls_through(None, Some(""), Some("c"), None, "c")]
#[case::empty_openai_key_falls_through(None, None, Some(""), Some("d"), "d")]
#[case::empty_environment_value_is_accepted(Some(""), None, None, Some(""), "")]
fn the_api_key_follows_the_or_chain(
    #[case] explicit: Option<&str>,
    #[case] global_api_key: Option<&str>,
    #[case] global_openai_key: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    let updated = validate_environment(
        &IndexMap::new(),
        &SpeechKeys {
            explicit: explicit.map(secret).as_ref(),
            global_api_key: global_api_key.map(secret).as_ref(),
            global_openai_key: global_openai_key.map(secret).as_ref(),
            environment: environment.map(secret).as_ref(),
        },
    )
    .expect("validated headers");

    assert_eq!(
        updated.get("xi-api-key").map(String::as_str),
        Some(expected)
    );
}

#[test]
fn a_missing_api_key_is_an_error() {
    let empty = secret("");
    let result = validate_environment(
        &IndexMap::new(),
        &SpeechKeys {
            explicit: Some(&empty),
            global_api_key: None,
            global_openai_key: None,
            environment: None,
        },
    );

    assert!(matches!(result, Err(SpeechError::ApiKeyRequired)));
}

#[test]
fn validating_the_environment_updates_two_headers_and_keeps_the_rest() {
    let key = secret("k");
    let given = headers(&[
        ("Content-Type", "text/plain"),
        ("content-type", "kept"),
        ("X-Trace", "1"),
    ]);
    let updated = validate_environment(
        &given,
        &SpeechKeys {
            explicit: Some(&key),
            global_api_key: None,
            global_openai_key: None,
            environment: None,
        },
    )
    .expect("validated headers");

    assert_eq!(
        updated,
        headers(&[
            ("Content-Type", "application/json"),
            ("content-type", "kept"),
            ("X-Trace", "1"),
            ("xi-api-key", "k"),
        ])
    );
    assert_eq!(
        given.get("Content-Type").map(String::as_str),
        Some("text/plain")
    );
}

#[test]
fn the_elevenlabs_error_displays_its_message() {
    let error = ElevenLabsError {
        status_code: 401,
        message: "bad key".to_owned(),
        headers: IndexMap::new(),
    };

    assert_eq!(error.to_string(), "bad key");
}
