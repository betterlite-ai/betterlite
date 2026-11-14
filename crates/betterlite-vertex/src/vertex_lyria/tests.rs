#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_models::catalog::BundledCatalog;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use tokio::runtime::Builder;

use super::error::VertexLyriaError;
use super::mapping::{DropParams, LyriaMapping, SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::request::{LyriaCall, LyriaRequest, prepare_request};
use crate::model_policy::lyria::LyriaOverlay;
use crate::token_cache::probe::AccessToken;
use crate::vertex_interactions::mint::{MintAccessToken, MintFuture};
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
        let $overlay = &LyriaOverlay {
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

#[rstest]
#[case::number(Json::Number(Number::from(0_u64)), "response_format=0")]
#[case::bool(Json::Bool(true), "response_format=true")]
#[case::list(Json::List(vec![text("wav")]), "response_format=a list")]
#[case::object(Json::Object(Object::new()), "response_format=an object")]
fn renders_a_non_string_format_in_the_error(#[case] value: Json, #[case] shown: &str) {
    let optional = params(&[("response_format", value)]);
    let error = mapped("lyria-002", &optional, None, DropParams::default()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains(&format!("does not support {shown}."))
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
    assert_eq!(mapping.params.len(), 1);
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
    assert_eq!(mapping.params, params(&[("response_format", text("wav"))]));
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
        serde_json::to_string(&request.body).unwrap(),
        r#"{"instances":[{"prompt":"draw a river"}],"parameters":{"sample_count":1}}"#
    );
}

#[test]
fn builds_the_interactions_body_with_a_wav_format() {
    let optional = params(&[("response_format", text("wav"))]);
    let request = prepared("lyria-3-pro-preview", &optional, None, None).unwrap();
    assert_eq!(
        serde_json::to_string(&request.body).unwrap(),
        r#"{"model":"lyria-3-pro-preview","input":"draw a river","response_format":{"type":"audio","mime_type":"audio/wav"}}"#
    );
}

#[test]
fn leaves_the_interactions_format_out_for_mp3() {
    let optional = params(&[("response_format", text("mp3"))]);
    let request = prepared("lyria-3-pro-preview", &optional, None, None).unwrap();
    assert_eq!(
        serde_json::to_string(&request.body).unwrap(),
        r#"{"model":"lyria-3-pro-preview","input":"draw a river"}"#
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

#[rstest]
#[case::predict("lyria-002")]
#[case::interactions("lyria-3-pro-preview")]
fn mints_once_and_reuses_the_token(#[case] model: &str) {
    let mint = FixedMint::new();
    let request = prepare_with(&mint, model, &Object::new(), &IndexMap::new()).unwrap();
    assert_eq!(mint.calls(), vec![(None, None)]);
    assert_eq!(
        request.headers["Authorization"].expose_secret(),
        "Bearer test-token"
    );
}

#[test]
fn interactions_mint_once_with_a_configured_project() {
    let input = "draw a river";
    let call = LyriaCall {
        model: "lyria-3-pro-preview",
        api_base: None,
        project: Some("team-project"),
        location: None,
        credentials: None,
        input,
        optional_params: &Object::new(),
    };
    let mint = FixedMint::new();
    let request =
        with_catalog!(|overlay| block_on(prepare_request(&mint, overlay, &call, &IndexMap::new())))
            .unwrap();
    assert_eq!(
        request.url,
        "https://aiplatform.googleapis.com/v1beta1/projects/team-project/locations/global/interactions"
    );
    assert_eq!(mint.calls(), vec![(None, Some("team-project".to_owned()))]);
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

mod decode;
