#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::{
    ConfiguredCredentials, CredentialSource,
};
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_config::sections::vertex_embedding_dispatch::Settings as EmbeddingSettings;
use betterlite_core::vertex_embedding_dispatch::{Defaults, Resolution, pin};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingResponse, EmbeddingUsage};
use betterlite_vertex::gemini_embedding_call::error::GeminiEmbeddingCallError;
use betterlite_vertex::gemini_embedding_records::input::{
    EmbeddingInput as EmbedContentInput, InputEntry,
};
use betterlite_vertex::gemini_embedding_records::response::EmbeddingReply;
use betterlite_vertex::gemini_embedding_records::usage::EmbeddingUsage as GeminiUsage;
use betterlite_vertex::multimodal_embedding_records::response::MultimodalEmbeddingAnswer;
use betterlite_vertex::multimodal_embedding_records::usage::MediaPromptTokensDetails;
use betterlite_vertex::vertex_embedding_call::error::VertexEmbeddingCallError;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};

use super::arguments::{EmbedContentArguments, MultimodalArguments, TextArguments};
use super::{
    EmbedContentOutcome, MultimodalOutcome, TextOutcome, VertexEmbeddingError,
    VertexEmbeddingHandlers, VertexEmbeddingInput, VertexEmbeddingPrepareError,
    VertexEmbeddingPrepared, VertexEmbeddingResult, execute, prepare,
};

const TEXT_MODEL: &str = "text-embedding-005";
const EMBED_CONTENT_MODEL: &str = "gemini-embedding-2";
const ACCOUNT: &str =
    r#"{"type": "service_account", "client_email": "sa@example.iam", "private_key": "hunter2"}"#;

fn catalog(source: &str) -> ModelCatalog {
    ModelCatalog::parse(source.as_bytes(), UnknownFields::Reject).expect("a catalog")
}

fn params(source: &str) -> Extra {
    serde_json::from_str(source).expect("optional params")
}

fn request(model: &str, input: EmbeddingInput, optional_params: &str) -> VertexEmbeddingInput {
    VertexEmbeddingInput {
        model: model.to_owned(),
        input,
        optional_params: params(optional_params),
        litellm_params: Some(params(r#"{"custom_llm_provider": "vertex_ai"}"#)),
        api_key: Some(SecretString::from("gemini-key".to_owned())),
        api_base: Some("https://vertex.example".to_owned()),
        pinned: None,
        extra_headers: BTreeMap::from([("x-goog-request-params".to_owned(), "a=b".to_owned())]),
        timeout: Some(Duration::from_secs(42)),
        aembedding: true,
    }
}

fn prepare_plain(
    request: VertexEmbeddingInput,
    catalog: &ModelCatalog,
) -> Result<VertexEmbeddingPrepared, VertexEmbeddingPrepareError> {
    let vertex = VertexSettings::default();
    let embedding = EmbeddingSettings::default();
    let defaults = Defaults {
        vertex: &vertex,
        embedding: &embedding,
    };
    prepare(request, defaults, catalog)
}

fn prepared(
    model: &str,
    input: EmbeddingInput,
    optional_params: &str,
    catalog: &ModelCatalog,
) -> VertexEmbeddingPrepared {
    prepare_plain(request(model, input, optional_params), catalog).expect("a prepared request")
}

fn labelled(call: &VertexEmbeddingPrepared) -> (&'static str, &Resolution) {
    match call {
        VertexEmbeddingPrepared::EmbedContent(arm) => ("embed_content", &arm.resolved),
        VertexEmbeddingPrepared::Multimodal(arm) => ("multimodal", &arm.resolved),
        VertexEmbeddingPrepared::Text(arm) => ("text", &arm.resolved),
    }
}

fn embed_content_catalog() -> ModelCatalog {
    catalog(&format!(
        r#"{{"vertex_ai/{EMBED_CONTENT_MODEL}": {{"litellm_provider": "vertex_ai", "mode": "embedding", "uses_embed_content": true}}}}"#
    ))
}

fn assert_text_arguments() {
    let VertexEmbeddingPrepared::Text(text) = prepared(
        TEXT_MODEL,
        EmbeddingInput::Text("hello".to_owned()),
        "{}",
        &ModelCatalog::default(),
    ) else {
        panic!("the text model selects the text prediction API");
    };
    let key = text.api_key.as_ref().expect("the supplied key");
    assert_eq!(key.expose_secret(), "gemini-key");
    assert_eq!(text.timeout, Some(Duration::from_secs(42)));
    assert_eq!(
        text.litellm_params,
        Some(params(r#"{"custom_llm_provider": "vertex_ai"}"#))
    );
    assert_eq!(text.resolved.optional_params, Extra::new());
    assert!(text.aembedding);
}

fn assert_embed_content_arguments() {
    let VertexEmbeddingPrepared::EmbedContent(media) = prepared(
        EMBED_CONTENT_MODEL,
        EmbeddingInput::Texts(vec!["one".to_owned(), "two".to_owned()]),
        r#"{"image": "gs://bucket/cat.png"}"#,
        &embed_content_catalog(),
    ) else {
        panic!("uses_embed_content selects the embedContent API");
    };
    assert_eq!(
        media.input,
        EmbedContentInput::Entries(vec![
            InputEntry::Text("one".to_owned()),
            InputEntry::Text("two".to_owned()),
        ])
    );
    assert_eq!(
        media
            .extra_headers
            .get("x-goog-request-params")
            .map(String::as_str),
        Some("a=b")
    );
    assert!(media.resolved.optional_params.contains_key("image"));
    assert!(media.aembedding);
}

type ResolvedFields<'call> = (
    &'static str,
    Option<&'call str>,
    Option<&'call str>,
    Option<&'call str>,
    Option<&'call str>,
);

fn resolved_fields(arm: &VertexEmbeddingPrepared) -> ResolvedFields<'_> {
    let (label, resolved) = labelled(arm);
    let key = resolved.credentials.as_ref();
    (
        label,
        resolved.project.as_deref(),
        resolved.location.as_deref(),
        key.and_then(ConfiguredCredentials::as_text),
        resolved.api_base.as_deref(),
    )
}

fn configured_settings() -> (VertexSettings, EmbeddingSettings) {
    let vertex = VertexSettings {
        project: Some("settings-project".to_owned()),
        location: Some("settings-location".to_owned()),
        credentials: Some(ConfiguredCredentials::Text(SecretString::from(
            "settings-key".to_owned(),
        ))),
        adc: AdcEnvironment::default(),
        default_video_duration_seconds: 8,
    };
    let embedding = EmbeddingSettings {
        project: None,
        credentials: None,
        api_base: Some("https://settings.example".to_owned()),
        deployments: Vec::new(),
    };
    (vertex, embedding)
}

#[test]
fn resolved_settings_reach_every_arm() {
    let (vertex, embedding) = configured_settings();
    let defaults = Defaults {
        vertex: &vertex,
        embedding: &embedding,
    };
    let cases = [
        (EMBED_CONTENT_MODEL, "{}", embed_content_catalog()),
        (TEXT_MODEL, r#"{"image": null}"#, ModelCatalog::default()),
        (TEXT_MODEL, "{}", ModelCatalog::default()),
    ];
    let arms: Vec<VertexEmbeddingPrepared> = cases
        .iter()
        .map(|(model, options, catalog)| {
            let hello = EmbeddingInput::Text("hello".to_owned());
            prepare(request(model, hello, options), defaults, catalog).expect("a prepared request")
        })
        .collect();
    let seen: Vec<ResolvedFields<'_>> = arms.iter().map(resolved_fields).collect();
    // the explicit api_base wins over the setting, as in main.py's or chain
    let expected = ["embed_content", "multimodal", "text"].map(|label| {
        (
            label,
            Some("settings-project"),
            Some("settings-location"),
            Some("settings-key"),
            Some("https://vertex.example"),
        )
    });
    assert_eq!(seen, expected);
}

#[rstest]
#[case::embed_content("embed_content", EMBED_CONTENT_MODEL, "", embed_content_catalog())]
#[case::multimodal(
    "multimodal",
    TEXT_MODEL,
    r#", "image": null"#,
    ModelCatalog::default()
)]
#[case::text("text", TEXT_MODEL, "", ModelCatalog::default())]
fn an_object_credential_reaches_every_arm(
    #[case] arm: &str,
    #[case] model: &str,
    #[case] media: &str,
    #[case] catalog: ModelCatalog,
) {
    let optional_params = format!(r#"{{"vertex_credentials": {ACCOUNT}{media}}}"#);
    let call = prepared(
        model,
        EmbeddingInput::Text("hello".to_owned()),
        &optional_params,
        &catalog,
    );
    let (selected, resolved) = labelled(&call);
    assert_eq!(selected, arm);
    assert!(matches!(
        resolved.credentials,
        Some(ConfiguredCredentials::Source(
            CredentialSource::ServiceAccount(_)
        ))
    ));
}

#[rstest]
#[case::embed_content("embed_content", EMBED_CONTENT_MODEL, "{}", embed_content_catalog())]
#[case::multimodal(
    "multimodal",
    TEXT_MODEL,
    r#"{"image": null}"#,
    ModelCatalog::default()
)]
#[case::text("text", TEXT_MODEL, "{}", ModelCatalog::default())]
fn pinned_options_reach_every_arm(
    #[case] arm: &str,
    #[case] model: &str,
    #[case] options: &str,
    #[case] catalog: ModelCatalog,
) {
    let pins = format!(
        r#"{{"vertex_project": "pinned-project", "vertex_ai_location": "pinned-location", "vertex_ai_credentials": {ACCOUNT}}}"#
    );
    let mut pinned_request = request(model, EmbeddingInput::Text("hello".to_owned()), options);
    pinned_request.pinned = Some(pin(&params(&pins)).expect("pinned options"));
    let call = prepare_plain(pinned_request, &catalog).expect("a prepared request");
    assert!(!format!("{call:?}").contains("hunter2"));
    let (selected, resolved) = labelled(&call);
    assert_eq!(selected, arm);
    assert_eq!(resolved.project.as_deref(), Some("pinned-project"));
    assert_eq!(resolved.location.as_deref(), Some("pinned-location"));
    assert!(matches!(
        resolved.credentials,
        Some(ConfiguredCredentials::Source(
            CredentialSource::ServiceAccount(_)
        ))
    ));
}

#[test]
fn branch_specific_arguments() {
    assert_text_arguments();
    assert_embed_content_arguments();
}

#[rstest]
#[case::null_image(
    EmbeddingInput::Text("hello".to_owned()),
    r#"{"image": null}"#,
    Json::Text("hello".to_owned())
)]
#[case::text_list(
    EmbeddingInput::Texts(vec!["one".to_owned(), "two".to_owned()]),
    r#"{"image": "gs://bucket/cat.png"}"#,
    Json::List(vec![Json::Text("one".to_owned()), Json::Text("two".to_owned())])
)]
#[case::token_list(
    EmbeddingInput::Tokens(vec![1, 2]),
    r#"{"image": "gs://bucket/cat.png"}"#,
    Json::List(vec![
        Json::Number(serde_json::Number::from(1_i32)),
        Json::Number(serde_json::Number::from(2_i32)),
    ])
)]
fn a_media_key_selects_multimodal_arguments(
    #[case] input: EmbeddingInput,
    #[case] options: &str,
    #[case] expected: Json,
) {
    let VertexEmbeddingPrepared::Multimodal(multimodal) =
        prepared(TEXT_MODEL, input, options, &ModelCatalog::default())
    else {
        panic!("a present media key selects multimodal prediction");
    };
    assert_eq!(multimodal.input, expected);
    assert!(multimodal.litellm_params.is_some());
    assert_eq!(
        multimodal.resolved.optional_params.get("image"),
        params(options).get("image")
    );
    assert!(multimodal.aembedding);
}

#[derive(Default)]
struct Recorder {
    called: Mutex<Vec<(&'static str, String)>>,
}

impl Recorder {
    fn record(&self, arm: &'static str, model: &str) {
        self.called
            .lock()
            .expect("the call log")
            .push((arm, model.to_owned()));
    }

    fn calls(&self) -> Vec<(&'static str, String)> {
        let calls = self.called.lock().expect("the call log");
        calls.clone()
    }
}

fn reply() -> EmbeddingReply {
    EmbeddingReply {
        model: EMBED_CONTENT_MODEL.to_owned(),
        data: Vec::new(),
        usage: GeminiUsage {
            prompt_tokens: 1,
            completion_tokens: 0,
            total_tokens: 1,
            prompt_tokens_details: None,
        },
    }
}

fn response() -> EmbeddingResponse {
    EmbeddingResponse {
        object: "list".to_owned(),
        data: Vec::new(),
        model: TEXT_MODEL.to_owned(),
        usage: EmbeddingUsage {
            prompt_tokens: 1,
            completion_tokens: 0,
            completion_tokens_details: None,
            prompt_tokens_details: None,
            total_tokens: 1,
        },
    }
}

fn answer() -> MultimodalEmbeddingAnswer {
    MultimodalEmbeddingAnswer {
        response: response(),
        details: MediaPromptTokensDetails {
            character_count: None,
            image_count: 0,
            video_length_seconds: Decimal::ZERO,
        },
    }
}

impl VertexEmbeddingHandlers for Recorder {
    fn embed_content(
        &self,
        call: &EmbedContentArguments,
    ) -> impl Future<Output = EmbedContentOutcome> + Send {
        self.record("embed_content", &call.model);
        ready(Ok(reply()))
    }

    fn multimodal(
        &self,
        call: &MultimodalArguments,
    ) -> impl Future<Output = MultimodalOutcome> + Send {
        self.record("multimodal", &call.model);
        ready(Ok(answer()))
    }

    fn text(&self, call: &TextArguments) -> impl Future<Output = TextOutcome> + Send {
        self.record("text", &call.model);
        ready(Ok(response()))
    }
}

#[rstest]
#[case::embed_content(
    EMBED_CONTENT_MODEL,
    EmbeddingInput::Text("hello".to_owned()),
    "{}",
    embed_content_catalog(),
    "embed_content"
)]
#[case::multimodal(
    TEXT_MODEL,
    EmbeddingInput::Text("hello".to_owned()),
    r#"{"video": null}"#,
    ModelCatalog::default(),
    "multimodal"
)]
#[case::text(
    TEXT_MODEL,
    EmbeddingInput::Tokens(vec![1, 2]),
    "{}",
    ModelCatalog::default(),
    "text"
)]
#[tokio::test]
async fn runs_each_arm_through_its_handler(
    #[case] model: &str,
    #[case] input: EmbeddingInput,
    #[case] options: &str,
    #[case] catalog: ModelCatalog,
    #[case] arm: &str,
) {
    let recorder = Recorder::default();
    let call = prepared(model, input, options, &catalog);
    let result = execute(&call, &recorder).await.expect("a handler result");
    let produced = match result {
        VertexEmbeddingResult::EmbedContent(_) => "embed_content",
        VertexEmbeddingResult::Multimodal(_) => "multimodal",
        VertexEmbeddingResult::Text(_) => "text",
    };
    assert_eq!(produced, arm);
    // the handler of the selected arm receives that arm's prepared call
    assert_eq!(recorder.calls(), [(arm, model.to_owned())]);
}

#[test]
fn token_input_is_rejected_for_embed_content() {
    let vertex = VertexSettings::default();
    let embedding = EmbeddingSettings::default();
    let error = prepare(
        request(
            EMBED_CONTENT_MODEL,
            EmbeddingInput::Tokens(vec![1, 2]),
            "{}",
        ),
        Defaults {
            vertex: &vertex,
            embedding: &embedding,
        },
        &embed_content_catalog(),
    )
    .expect_err("tokens have no embedContent form");
    assert!(matches!(error, VertexEmbeddingPrepareError::TokenInput));
}

struct Failing;

impl VertexEmbeddingHandlers for Failing {
    fn embed_content(
        &self,
        _call: &EmbedContentArguments,
    ) -> impl Future<Output = EmbedContentOutcome> + Send {
        ready(Err(GeminiEmbeddingCallError::MissingApiKey))
    }

    fn multimodal(
        &self,
        _call: &MultimodalArguments,
    ) -> impl Future<Output = MultimodalOutcome> + Send {
        ready(Err(VertexEmbeddingCallError::Timeout))
    }

    fn text(&self, _call: &TextArguments) -> impl Future<Output = TextOutcome> + Send {
        ready(Err(VertexEmbeddingCallError::Status {
            status: 429,
            body: br#"{"error": {"message": "quota"}}"#.to_vec(),
        }))
    }
}

#[tokio::test]
async fn handler_errors_keep_their_variant() {
    let text = prepared(
        TEXT_MODEL,
        EmbeddingInput::Text("hello".to_owned()),
        "{}",
        &ModelCatalog::default(),
    );
    let Err(VertexEmbeddingError::Prediction(VertexEmbeddingCallError::Status { status, .. })) =
        execute(&text, &Failing).await
    else {
        panic!("the text handler's status error travels unchanged");
    };
    assert_eq!(status, 429);
}
