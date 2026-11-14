#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::{Future, ready};
use std::path::PathBuf;
use std::sync::Mutex;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_models::catalog::BundledCatalog;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use tokio::runtime::Builder;

use super::audio::{DEFAULT_SPEECH_MEDIA_TYPE, LyriaDecode, decode_lyria_audio};
use super::error::VertexLyriaError;
use super::mapping::{DropParams, LyriaMapping, SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::request::{LyriaCall, LyriaRequest, prepare_request};
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::model_policy::lyria::LyriaOverlay;
use crate::token_cache::probe::AccessToken;
use crate::vertex_interactions::mint::{MintAccessToken, MintFuture};
use crate::vertex_speech::audio::BinaryAudio;
use crate::vertex_speech::mapping::SpeechVoice;

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

    fn with_project(project: &'static str) -> Self {
        Self {
            project,
            ..Self::new()
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
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

macro_rules! with_catalog {
    (|$overlay:ident| $body:expr) => {{
        let runtime = BTreeMap::new();
        let catalog = BundledCatalog::default().get();
        let $overlay = LyriaOverlay {
            runtime: &runtime,
            bundled: catalog.as_ref(),
        };
        $body
    }};
}

fn params(entries: &[(&str, Json)]) -> Object {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn mapped(
    model: &str,
    optional: &Object,
    voice: Option<&SpeechVoice>,
    drop: DropParams,
) -> Result<LyriaMapping, VertexLyriaError> {
    with_catalog!(|overlay| map_openai_params(overlay, model, optional, voice, drop))
}

fn prepared(
    model: &str,
    optional: &Object,
    project: Option<&str>,
    location: Option<&str>,
) -> Result<LyriaRequest, VertexLyriaError> {
    let input = "draw a river";
    let call = LyriaCall {
        model,
        api_base: None,
        project,
        location,
        credentials: None,
        input,
        optional_params: optional,
    };
    with_catalog!(|overlay| {
        let mint = FixedMint::new();
        block_on(prepare_request(&mint, overlay, &call, &IndexMap::new()))
    })
}

fn prepare_with(
    mint: &dyn MintAccessToken,
    model: &str,
    optional: &Object,
    caller: &IndexMap<String, String>,
) -> Result<LyriaRequest, VertexLyriaError> {
    let input = "draw a river";
    let call = LyriaCall {
        model,
        api_base: None,
        project: None,
        location: None,
        credentials: None,
        input,
        optional_params: optional,
    };
    with_catalog!(|overlay| block_on(prepare_request(mint, overlay, &call, caller)))
}

fn decode(model: &str, body: &str) -> Result<BinaryAudio, VertexLyriaError> {
    with_catalog!(|overlay| {
        let decode = LyriaDecode {
            model,
            status: 200,
            body: body.as_bytes(),
        };
        decode_lyria_audio(overlay, &decode, &sniff_flac)
    })
}

fn sniff_flac(audio: &[u8]) -> Option<String> {
    audio.starts_with(b"fLaC").then(|| "audio/flac".to_owned())
}

#[test]
fn advertises_only_response_format() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["response_format"]);
}

#[rstest]
#[case::predict_wav("lyria-002", "wav")]
#[case::clip_mp3("lyria-3-clip-preview", "mp3")]
#[case::pro_mp3("lyria-3-pro-preview", "mp3")]
#[case::pro_wav("lyria-3-pro-preview", "wav")]
#[case::prefixed_model("vertex_ai/lyria-002", "wav")]
fn keeps_a_format_the_catalog_declares(#[case] model: &str, #[case] format: &str) {
    let optional = params(&[("response_format", text(format))]);
    let mapping = mapped(model, &optional, None, DropParams::default()).unwrap();
    assert_eq!(mapping.params.get("response_format"), Some(&text(format)));
}

#[rstest]
#[case::predict_mp3("lyria-002", "mp3", "wav")]
#[case::clip_wav("lyria-3-clip-preview", "wav", "mp3")]
#[case::unknown_format("lyria-002", "ogg", "wav")]
#[case::pro_unknown("lyria-3-pro-preview", "flac", "mp3, wav")]
#[case::empty_format("lyria-002", "", "wav")]
fn rejects_a_format_the_catalog_does_not_declare(
    #[case] model: &str,
    #[case] format: &str,
    #[case] supported: &str,
) {
    let optional = params(&[("response_format", text(format))]);
    let error = mapped(model, &optional, None, DropParams::default()).unwrap_err();
    assert!(matches!(error, VertexLyriaError::UnsupportedFormat { .. }));
    assert_eq!(error.status(), Some(400));
    assert!(
        error
            .to_string()
            .contains(&format!("Supported values: {supported}."))
    );
}

#[test]
fn names_the_prefix_stripped_model_in_the_format_error() {
    let optional = params(&[("response_format", text("mp3"))]);
    let error = mapped(
        "vertex_ai/lyria-002",
        &optional,
        None,
        DropParams::default(),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("Vertex AI lyria-002 does not support response_format='mp3'.")
    );
}

#[rstest]
#[case::call(DropParams { call: true, global: false })]
#[case::global(DropParams { call: false, global: true })]
#[case::both(DropParams { call: true, global: true })]
fn drops_an_undeclared_format(#[case] drop: DropParams) {
    let optional = params(&[
        ("response_format", text("mp3")),
        ("temperature", Json::Number(Number::from(1_u64))),
    ]);
    let mapping = mapped("lyria-002", &optional, None, drop).unwrap();
    assert_eq!(mapping.params.get("response_format"), None);
    assert!(mapping.params.contains_key("temperature"));
}

#[rstest]
#[case::speed(text("1.5"), "speed")]
#[case::instructions(text("whisper"), "instructions")]
#[case::zero_speed(Json::Number(Number::from(0_u64)), "speed")]
#[case::false_speed(Json::Bool(false), "speed")]
#[case::empty_speed(text(""), "speed")]
fn rejects_an_unsupported_parameter(#[case] value: Json, #[case] name: &str) {
    let optional = params(&[(name, value)]);
    let error = mapped("lyria-002", &optional, None, DropParams::default()).unwrap_err();
    assert!(matches!(error, VertexLyriaError::UnsupportedParams { .. }));
    assert_eq!(error.status(), Some(400));
    assert!(
        error
            .to_string()
            .contains(&format!("does not support the OpenAI parameters: {name}."))
    );
}

#[test]
fn names_both_unsupported_parameters_in_order() {
    let optional = params(&[("speed", text("1.5")), ("instructions", text("whisper"))]);
    let error = mapped("lyria-002", &optional, None, DropParams::default()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not support the OpenAI parameters: speed, instructions.")
    );
}

#[test]
fn reports_only_the_unsupported_parameter_that_is_set() {
    let optional = params(&[("speed", Json::Null), ("instructions", text("whisper"))]);
    let error = mapped("lyria-002", &optional, None, DropParams::default()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not support the OpenAI parameters: instructions.")
    );
}

#[rstest]
#[case::call(DropParams { call: true, global: false })]
#[case::global(DropParams { call: false, global: true })]
#[case::both(DropParams { call: true, global: true })]
fn drops_the_unsupported_parameters(#[case] drop: DropParams) {
    let optional = params(&[
        ("speed", text("1.5")),
        ("instructions", text("whisper")),
        ("response_format", text("wav")),
    ]);
    let mapping = mapped("lyria-002", &optional, None, drop).unwrap();
    assert_eq!(mapping.params.get("speed"), None);
    assert_eq!(mapping.params.get("instructions"), None);
    assert_eq!(mapping.params.get("response_format"), Some(&text("wav")));
}

#[test]
fn keeps_a_null_parameter_as_given() {
    let optional = params(&[("speed", Json::Null)]);
    let mapping = mapped("lyria-002", &optional, None, DropParams::default()).unwrap();
    assert_eq!(mapping.params.get("speed"), Some(&Json::Null));
}

#[test]
fn copies_every_other_optional_parameter() {
    let optional = params(&[
        ("response_format", text("wav")),
        ("temperature", Json::Number(Number::from(1_u64))),
        ("extra", Json::List(vec![text("a")])),
    ]);
    let mapping = mapped("lyria-002", &optional, None, DropParams::default()).unwrap();
    assert_eq!(mapping.params, optional);
}

#[test]
fn rejects_an_unknown_model() {
    let error = mapped("lyria-9", &Object::new(), None, DropParams::default()).unwrap_err();
    assert!(matches!(error, VertexLyriaError::UnknownModel { .. }));
    assert_eq!(error.status(), Some(500));
    assert!(error.to_string().contains("\"lyria-9\""));
}

#[test]
fn rejects_an_unknown_model_before_reading_its_parameters() {
    let optional = params(&[("speed", text("1.5"))]);
    let error = mapped("lyria-9", &optional, None, DropParams::default()).unwrap_err();
    assert!(matches!(error, VertexLyriaError::UnknownModel { .. }));
}

#[test]
fn returns_a_voice_only_when_it_is_a_string() {
    let named = SpeechVoice::Name("alloy".to_owned());
    assert_eq!(
        mapped(
            "lyria-002",
            &Object::new(),
            Some(&named),
            DropParams::default()
        )
        .unwrap()
        .voice,
        Some("alloy".to_owned())
    );
    let options = SpeechVoice::Options(params(&[("name", text("custom"))]));
    assert_eq!(
        mapped(
            "lyria-002",
            &Object::new(),
            Some(&options),
            DropParams::default()
        )
        .unwrap()
        .voice,
        None
    );
    assert_eq!(
        mapped("lyria-002", &Object::new(), None, DropParams::default())
            .unwrap()
            .voice,
        None
    );
}

#[test]
fn builds_one_prompt_instance_and_sample_count_one() {
    let optional = params(&[("response_format", text("wav"))]);
    let request = prepared("lyria-002", &optional, None, None).unwrap();
    assert_eq!(
        serde_json::to_value(&request.body).unwrap(),
        serde_json::json!({
            "instances": [{"prompt": "draw a river"}],
            "parameters": {"sample_count": 1},
        })
    );
}

#[test]
fn builds_the_interactions_body_with_a_wav_format() {
    let optional = params(&[("response_format", text("wav"))]);
    let request = prepared("lyria-3-pro-preview", &optional, None, None).unwrap();
    assert_eq!(
        serde_json::to_value(&request.body).unwrap(),
        serde_json::json!({
            "model": "lyria-3-pro-preview",
            "input": "draw a river",
            "response_format": {"type": "audio", "mime_type": "audio/wav"},
        })
    );
}

#[test]
fn leaves_the_interactions_format_out_for_mp3() {
    let optional = params(&[("response_format", text("mp3"))]);
    let request = prepared("lyria-3-pro-preview", &optional, None, None).unwrap();
    assert_eq!(
        serde_json::to_value(&request.body).unwrap(),
        serde_json::json!({
            "model": "lyria-3-pro-preview",
            "input": "draw a river",
        })
    );
}

#[test]
fn leaves_the_interactions_format_out_without_a_response_format() {
    let request = prepared("lyria-3-pro-preview", &Object::new(), None, None).unwrap();
    assert_eq!(
        serde_json::to_value(&request.body)
            .unwrap()
            .get("response_format"),
        None
    );
}

#[test]
fn builds_the_predict_url_at_the_default_location() {
    let request = prepared("lyria-002", &Object::new(), None, None).unwrap();
    assert_eq!(
        request.url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/project-1/locations/us-central1/publishers/google/models/lyria-002:predict"
    );
}

#[test]
fn uses_the_configured_project_and_location() {
    let request = prepared(
        "lyria-002",
        &Object::new(),
        Some("team-project"),
        Some("europe-west4"),
    )
    .unwrap();
    assert_eq!(
        request.url,
        "https://europe-west4-aiplatform.googleapis.com/v1/projects/team-project/locations/europe-west4/publishers/google/models/lyria-002:predict"
    );
    assert_eq!(
        request.headers["x-goog-user-project"].expose_secret(),
        "team-project"
    );
}

#[test]
fn encodes_the_predict_path_segments() {
    let request = prepared(
        "lyria-002",
        &Object::new(),
        Some("team/project"),
        Some("us-central1"),
    )
    .unwrap();
    assert!(
        request
            .url
            .contains("/v1/projects/team%2Fproject/locations/us-central1/")
    );
}

#[test]
fn rejects_a_project_that_cannot_be_a_path_segment() {
    let error = prepared("lyria-002", &Object::new(), Some(".."), None).unwrap_err();
    assert!(matches!(error, VertexLyriaError::Segment { .. }));
    assert_eq!(error.status(), Some(500));
}

#[test]
fn strips_one_vertex_prefix_from_the_model_path() {
    let request = prepared("vertex_ai/lyria-002", &Object::new(), None, None).unwrap();
    assert!(
        request
            .url
            .ends_with("/publishers/google/models/lyria-002:predict")
    );
}

#[test]
fn keeps_a_custom_api_base_and_trims_its_trailing_slash() {
    let input = "draw a river";
    let call = LyriaCall {
        model: "lyria-002",
        api_base: Some("https://host/proxy/"),
        project: None,
        location: None,
        credentials: None,
        input,
        optional_params: &Object::new(),
    };
    let request = with_catalog!(|overlay| {
        let mint = FixedMint::new();
        block_on(prepare_request(&mint, overlay, &call, &IndexMap::new()))
    })
    .unwrap();
    assert_eq!(
        request.url,
        "https://host/proxy/v1/projects/project-1/locations/us-central1/publishers/google/models/lyria-002:predict"
    );
}

#[test]
fn treats_an_empty_location_as_the_default() {
    let request = prepared("lyria-002", &Object::new(), None, Some("")).unwrap();
    assert!(request.url.contains("/locations/us-central1/"));
}

#[test]
fn rejects_an_invalid_location() {
    let error = prepared("lyria-002", &Object::new(), None, Some("Bad_Location")).unwrap_err();
    assert!(matches!(error, VertexLyriaError::ApiBase(_)));
    assert_eq!(error.status(), Some(500));
}

#[test]
fn builds_the_interactions_url_at_global() {
    let request = prepared("lyria-3-pro-preview", &Object::new(), None, None).unwrap();
    assert_eq!(
        request.url,
        "https://aiplatform.googleapis.com/v1beta1/projects/project-1/locations/global/interactions"
    );
}

#[test]
fn interactions_ignore_a_configured_location() {
    let request = prepared(
        "lyria-3-pro-preview",
        &Object::new(),
        None,
        Some("us-central1"),
    )
    .unwrap();
    assert_eq!(
        request.url,
        "https://aiplatform.googleapis.com/v1beta1/projects/project-1/locations/global/interactions"
    );
}

#[test]
fn interactions_keep_a_custom_api_base() {
    let input = "draw a river";
    let call = LyriaCall {
        model: "lyria-3-pro-preview",
        api_base: Some("https://host/proxy"),
        project: Some("team-project"),
        location: None,
        credentials: None,
        input,
        optional_params: &Object::new(),
    };
    let request = with_catalog!(|overlay| {
        let mint = FixedMint::new();
        block_on(prepare_request(&mint, overlay, &call, &IndexMap::new()))
    })
    .unwrap();
    assert_eq!(
        request.url,
        "https://host/proxy/v1beta1/projects/team-project/locations/global/interactions"
    );
}

#[test]
fn interactions_do_not_encode_the_project() {
    let input = "draw a river";
    let call = LyriaCall {
        model: "lyria-3-pro-preview",
        api_base: None,
        project: Some("team/project"),
        location: None,
        credentials: None,
        input,
        optional_params: &Object::new(),
    };
    let request = with_catalog!(|overlay| {
        let mint = FixedMint::new();
        block_on(prepare_request(&mint, overlay, &call, &IndexMap::new()))
    })
    .unwrap();
    assert!(
        request
            .url
            .contains("/v1beta1/projects/team/project/locations/global/")
    );
}

#[test]
fn mints_once_and_reuses_the_token() {
    let mint = FixedMint::new();
    let request = prepare_with(&mint, "lyria-002", &Object::new(), &IndexMap::new()).unwrap();
    assert_eq!(mint.calls(), vec![(None, None)]);
    assert_eq!(
        request.headers["Authorization"].expose_secret(),
        "Bearer test-token"
    );
}

#[test]
fn uses_the_minted_project_when_none_is_configured() {
    let mint = FixedMint::with_project("minted-project");
    let request = prepare_with(&mint, "lyria-002", &Object::new(), &IndexMap::new()).unwrap();
    assert!(request.url.contains("/v1/projects/minted-project/"));
    assert_eq!(
        request.headers["x-goog-user-project"].expose_secret(),
        "minted-project"
    );
}

#[test]
fn treats_an_empty_project_as_unset() {
    let mint = FixedMint::with_project("minted-project");
    let input = "draw a river";
    let call = LyriaCall {
        model: "lyria-002",
        api_base: None,
        project: Some(""),
        location: None,
        credentials: None,
        input,
        optional_params: &Object::new(),
    };
    let request =
        with_catalog!(|overlay| block_on(prepare_request(&mint, overlay, &call, &IndexMap::new())))
            .unwrap();
    assert!(request.url.contains("/v1/projects/minted-project/"));
    assert_eq!(mint.calls(), vec![(None, None)]);
}

#[test]
fn prefers_the_configured_project_over_the_mint() {
    let mint = FixedMint::new();
    let input = "draw a river";
    let project = "configured-project";
    let call = LyriaCall {
        model: "lyria-002",
        api_base: None,
        project: Some(project),
        location: None,
        credentials: None,
        input,
        optional_params: &Object::new(),
    };
    let request =
        with_catalog!(|overlay| block_on(prepare_request(&mint, overlay, &call, &IndexMap::new())))
            .unwrap();
    assert!(request.url.contains("/v1/projects/configured-project/"));
    assert_eq!(
        mint.calls(),
        vec![(None, Some("configured-project".to_owned()))]
    );
}

#[test]
fn rejects_an_interactions_call_without_a_project() {
    let mint = FixedMint::with_project("");
    let error = prepare_with(
        &mint,
        "lyria-3-pro-preview",
        &Object::new(),
        &IndexMap::new(),
    )
    .unwrap_err();
    assert!(matches!(error, VertexLyriaError::Interactions(_)));
}

#[test]
fn rejects_an_unknown_model_before_minting() {
    let mint = FixedMint::new();
    let error = prepare_with(&mint, "lyria-9", &Object::new(), &IndexMap::new()).unwrap_err();
    assert!(matches!(error, VertexLyriaError::UnknownModel { .. }));
    assert!(mint.calls().is_empty());
}

#[test]
fn forces_json_authentication_and_the_project_header() {
    let mint = FixedMint::new();
    let mut caller = IndexMap::new();
    caller.insert("Authorization".to_owned(), "caller-secret".to_owned());
    caller.insert(
        "x-goog-user-project".to_owned(),
        "caller-project".to_owned(),
    );
    caller.insert("Content-Type".to_owned(), "text/plain".to_owned());
    caller.insert("charset".to_owned(), "latin-1".to_owned());
    let request = prepare_with(&mint, "lyria-002", &Object::new(), &caller).unwrap();
    assert_eq!(
        request.headers["Authorization"].expose_secret(),
        "Bearer test-token"
    );
    assert_eq!(
        request.headers["x-goog-user-project"].expose_secret(),
        "project-1"
    );
    assert_eq!(
        request.headers["Content-Type"].expose_secret(),
        "application/json"
    );
    assert_eq!(request.headers["charset"].expose_secret(), "UTF-8");
    assert_eq!(caller["Content-Type"], "text/plain");
}

#[test]
fn redacts_header_values_in_debug() {
    let mint = FixedMint::new();
    let mut caller = IndexMap::new();
    caller.insert("x-api-key".to_owned(), "caller-secret".to_owned());
    let request = prepare_with(&mint, "lyria-002", &Object::new(), &caller).unwrap();
    let rendered = format!("{request:?}");
    assert!(!rendered.contains("test-token"));
    assert!(!rendered.contains("caller-secret"));
}

#[test]
fn decodes_the_first_prediction_only() {
    let body = r#"{"predictions": [
        {"audioContent": "QUJD", "mimeType": "audio/wav"},
        {"audioContent": "WFk=", "mimeType": "audio/mpeg"}
    ]}"#;
    let audio = decode("lyria-002", body).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"ABC");
    assert_eq!(audio.content_type.as_deref(), Some("audio/wav"));
    assert_eq!(audio.status, 200);
}

#[test]
fn prefers_audio_content_over_the_base64_field() {
    let body = r#"{"predictions": [{"audioContent": "QUJD", "bytesBase64Encoded": "WFk="}]}"#;
    assert_eq!(decode("lyria-002", body).unwrap().bytes.as_slice(), b"ABC");
}

#[test]
fn falls_back_to_the_base64_field() {
    let body = r#"{"predictions": [{"bytesBase64Encoded": "WFk="}]}"#;
    assert_eq!(decode("lyria-002", body).unwrap().bytes.as_slice(), b"XY");
}

#[test]
fn falls_back_to_the_base64_field_when_audio_content_is_empty() {
    let body = r#"{"predictions": [{"audioContent": "", "bytesBase64Encoded": "QUJD"}]}"#;
    assert_eq!(decode("lyria-002", body).unwrap().bytes.as_slice(), b"ABC");
}

#[rstest]
#[case::no_predictions("{}")]
#[case::empty_predictions(r#"{"predictions": []}"#)]
#[case::null_predictions(r#"{"predictions": null}"#)]
#[case::no_audio(r#"{"predictions": [{"mimeType": "audio/wav"}]}"#)]
#[case::empty_audio(r#"{"predictions": [{"audioContent": ""}]}"#)]
#[case::null_audio(r#"{"predictions": [{"audioContent": null}]}"#)]
fn rejects_a_prediction_without_audio(#[case] body: &str) {
    let error = decode("lyria-002", body).unwrap_err();
    assert!(matches!(error, VertexLyriaError::MissingAudio { .. }));
    assert_eq!(error.status(), Some(500));
}

#[test]
fn names_the_prefix_stripped_model_in_the_missing_audio_error() {
    let error = decode("vertex_ai/lyria-002", "{}").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("no generated audio found in Vertex AI lyria-002 response")
    );
}

#[test]
fn falls_back_to_the_sniffer_for_an_empty_mime_type() {
    let body = r#"{"predictions": [{"audioContent": "ZkxhQwA=", "mimeType": ""}]}"#;
    let audio = decode("lyria-002", body).unwrap();
    assert_eq!(audio.content_type.as_deref(), Some("audio/flac"));
}

#[test]
fn falls_back_to_the_provider_mime_type() {
    let body = r#"{"predictions": [{"audioContent": "ZkxhQwA=", "mimeType": "audio/wav"}]}"#;
    let audio = decode("lyria-002", body).unwrap();
    assert_eq!(audio.content_type.as_deref(), Some("audio/wav"));
}

#[test]
fn keeps_the_provider_mime_type_on_an_interaction_audio_item() {
    let body = r#"{"steps": [{"type": "audio", "data": "ZkxhQwA=", "mime_type": "audio/wav"}]}"#;
    let audio = decode("lyria-3-pro-preview", body).unwrap();
    assert_eq!(audio.content_type.as_deref(), Some("audio/wav"));
}

#[test]
fn defaults_to_audio_mpeg_without_a_sniff_match() {
    let body = r#"{"predictions": [{"audioContent": "QUJD"}]}"#;
    let audio = decode("lyria-002", body).unwrap();
    assert_eq!(
        audio.content_type.as_deref(),
        Some(DEFAULT_SPEECH_MEDIA_TYPE)
    );
}

#[test]
fn preserves_the_upstream_status() {
    let body = r#"{"predictions": [{"audioContent": "QUJD"}]}"#;
    let audio = with_catalog!(|overlay| {
        let decode = LyriaDecode {
            model: "lyria-002",
            status: 503,
            body: body.as_bytes(),
        };
        decode_lyria_audio(overlay, &decode, &sniff_flac)
    })
    .unwrap();
    assert_eq!(audio.status, 503);
}

#[test]
fn rejects_an_unknown_model_when_decoding() {
    let error = decode("lyria-9", "{}").unwrap_err();
    assert!(matches!(error, VertexLyriaError::UnknownModel { .. }));
}

#[test]
fn rejects_a_response_over_the_limit() {
    let body = vec![b' '; MAX_RESPONSE_BYTES.saturating_add(1)];
    let error = with_catalog!(|overlay| {
        let decode = LyriaDecode {
            model: "lyria-002",
            status: 200,
            body: body.as_slice(),
        };
        decode_lyria_audio(overlay, &decode, &sniff_flac)
    })
    .unwrap_err();
    assert!(matches!(error, VertexLyriaError::ResponseTooLarge { .. }));
}

#[test]
fn keeps_a_body_of_exactly_the_limit() {
    let body = vec![b' '; MAX_RESPONSE_BYTES];
    let error = with_catalog!(|overlay| {
        let decode = LyriaDecode {
            model: "lyria-002",
            status: 200,
            body: body.as_slice(),
        };
        decode_lyria_audio(overlay, &decode, &sniff_flac)
    })
    .unwrap_err();
    assert!(matches!(error, VertexLyriaError::ResponseJson(_)));
}

#[rstest]
#[case::malformed("{nope")]
#[case::list("[]")]
#[case::text(r#""text""#)]
fn rejects_a_body_that_is_not_a_response_object(#[case] body: &str) {
    let error = decode("lyria-002", body).unwrap_err();
    assert!(matches!(error, VertexLyriaError::ResponseJson(_)));
}

#[test]
fn takes_the_last_audio_of_the_steps() {
    let body = r#"{"steps": [
        {"type": "audio", "data": "QUJD", "mime_type": "audio/mp3"},
        {"type": "audio", "data": "WFk=", "mime_type": "audio/wav"}
    ]}"#;
    let audio = decode("lyria-3-pro-preview", body).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"XY");
    assert_eq!(audio.content_type.as_deref(), Some("audio/wav"));
}

#[test]
fn scans_outputs_when_steps_is_absent() {
    let body = r#"{"outputs": [{"type": "audio", "data": "QUJD"}]}"#;
    let audio = decode("lyria-3-pro-preview", body).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"ABC");
    assert_eq!(
        audio.content_type.as_deref(),
        Some(DEFAULT_SPEECH_MEDIA_TYPE)
    );
}

#[test]
fn scans_outputs_when_steps_is_empty() {
    let body = r#"{"steps": [], "outputs": [{"type": "audio", "data": "QUJD"}]}"#;
    let audio = decode("lyria-3-pro-preview", body).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"ABC");
}

#[test]
fn prefers_steps_over_outputs() {
    let body = r#"{
        "steps": [{"type": "audio", "data": "QUJD"}],
        "outputs": [{"type": "audio", "data": "WFk="}]
    }"#;
    assert_eq!(
        decode("lyria-3-pro-preview", body)
            .unwrap()
            .bytes
            .as_slice(),
        b"ABC"
    );
}

#[test]
fn descends_the_content_of_a_model_output_step() {
    let body = r#"{"steps": [{
        "type": "model_output",
        "content": [
            {"type": "text", "data": "ignored"},
            {"type": "audio", "data": "QUJD", "mime_type": "audio/wav"}
        ]
    }]}"#;
    let audio = decode("lyria-3-pro-preview", body).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"ABC");
    assert_eq!(audio.content_type.as_deref(), Some("audio/wav"));
}

#[test]
fn treats_a_step_with_another_type_as_its_own_audio_item() {
    let body = r#"{"steps": [{"type": "audio", "data": "QUJD", "mime_type": "audio/wav"}]}"#;
    let audio = decode("lyria-3-pro-preview", body).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"ABC");
    assert_eq!(audio.content_type.as_deref(), Some("audio/wav"));
}

#[test]
fn skips_items_that_are_not_audio_or_carry_no_data() {
    let body = r#"{"steps": [
        {"type": "text", "data": "QUJD"},
        {"type": "audio", "data": ""},
        {"type": "audio"},
        {"type": "model_output", "content": [{"type": "text", "data": "WFk="}]}
    ]}"#;
    let error = decode("lyria-3-pro-preview", body).unwrap_err();
    assert!(matches!(error, VertexLyriaError::MissingAudio { .. }));
}

#[test]
fn resets_the_mime_type_with_the_last_audio() {
    let body = r#"{"steps": [
        {"type": "audio", "data": "QUJD", "mime_type": "audio/mp3"},
        {"type": "audio", "data": "ZkxhQwA="}
    ]}"#;
    let audio = decode("lyria-3-pro-preview", body).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"fLaC\x00");
    assert_eq!(audio.content_type.as_deref(), Some("audio/flac"));
}

#[rstest]
#[case::empty("{}")]
#[case::empty_steps(r#"{"steps": []}"#)]
#[case::null_steps(r#"{"steps": null}"#)]
#[case::audio_without_data(r#"{"steps": [{"type": "audio"}]}"#)]
fn rejects_an_interaction_response_without_audio(#[case] body: &str) {
    let error = decode("lyria-3-pro-preview", body).unwrap_err();
    assert!(matches!(error, VertexLyriaError::MissingAudio { .. }));
}

#[test]
fn uses_the_interactions_dialect_for_a_prefixed_model() {
    let body = r#"{"outputs": [{"type": "audio", "data": "QUJD"}]}"#;
    assert_eq!(
        decode("vertex_ai/lyria-3-pro-preview", body)
            .unwrap()
            .bytes
            .as_slice(),
        b"ABC"
    );
}

#[rstest]
fn replays_the_lyria_audio_corpus(#[files("fuzz/corpus/lyria_audio/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    with_catalog!(|overlay| {
        for model in ["lyria-002", "lyria-3-pro-preview"] {
            let decode = LyriaDecode {
                model,
                status: 200,
                body: &bytes,
            };
            drop(decode_lyria_audio(overlay, &decode, &sniff_flac));
        }
    });
}
