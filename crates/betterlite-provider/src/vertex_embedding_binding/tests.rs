#![cfg(test)]

use std::collections::BTreeMap;
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
        VertexEmbeddingPrepared::EmbedContent(call) => ("embed_content", &call.resolved),
        VertexEmbeddingPrepared::Multimodal(call) => ("multimodal", &call.resolved),
        VertexEmbeddingPrepared::Text(call) => ("text", &call.resolved),
    }
}

fn embed_content_catalog() -> ModelCatalog {
    catalog(&format!(
        r#"{{"vertex_ai/{EMBED_CONTENT_MODEL}": {{"litellm_provider": "vertex_ai", "mode": "embedding", "uses_embed_content": true}}}}"#
    ))
}

fn assert_text_arguments() {
    let text = prepared(
        TEXT_MODEL,
        EmbeddingInput::Text("hello".to_owned()),
        "{}",
        &ModelCatalog::default(),
    );
    let VertexEmbeddingPrepared::Text(text) = text else {
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
    let media = prepared(
        EMBED_CONTENT_MODEL,
        EmbeddingInput::Texts(vec!["one".to_owned(), "two".to_owned()]),
        r#"{"image": "gs://bucket/cat.png"}"#,
        &embed_content_catalog(),
    );
    let VertexEmbeddingPrepared::EmbedContent(media) = media else {
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

fn assert_multimodal_arguments() {
    let multimodal = prepared(
        TEXT_MODEL,
        EmbeddingInput::Text("hello".to_owned()),
        r#"{"image": null}"#,
        &ModelCatalog::default(),
    );
    let VertexEmbeddingPrepared::Multimodal(multimodal) = multimodal else {
        panic!("a present media key selects multimodal prediction");
    };
    assert_eq!(multimodal.input, Json::Text("hello".to_owned()));
    assert!(multimodal.litellm_params.is_some());
    assert_eq!(
        multimodal.resolved.optional_params.get("image"),
        Some(&Json::Null)
    );
    assert!(multimodal.aembedding);

    let texts = prepared(
        TEXT_MODEL,
        EmbeddingInput::Texts(vec!["one".to_owned(), "two".to_owned()]),
        r#"{"image": "gs://bucket/cat.png"}"#,
        &ModelCatalog::default(),
    );
    let VertexEmbeddingPrepared::Multimodal(texts) = texts else {
        panic!("a text list with a media key selects multimodal prediction");
    };
    assert_eq!(
        texts.input,
        Json::List(vec![
            Json::Text("one".to_owned()),
            Json::Text("two".to_owned()),
        ])
    );

    let tokens = prepared(
        TEXT_MODEL,
        EmbeddingInput::Tokens(vec![1, 2]),
        r#"{"image": "gs://bucket/cat.png"}"#,
        &ModelCatalog::default(),
    );
    let VertexEmbeddingPrepared::Multimodal(tokens) = tokens else {
        panic!("a token list with a media key selects multimodal prediction");
    };
    assert_eq!(
        tokens.input,
        Json::List(vec![
            Json::Number(serde_json::Number::from(1_i32)),
            Json::Number(serde_json::Number::from(2_i32)),
        ])
    );
}

#[test]
fn resolved_settings_reach_every_arm() {
    let vertex = VertexSettings {
        project: Some("settings-project".to_owned()),
        location: Some("settings-location".to_owned()),
        credentials: Some(ConfiguredCredentials::Text(SecretString::from(
            "settings-key".to_owned(),
        ))),
        adc: AdcEnvironment::default(),
    };
    let embedding = EmbeddingSettings {
        project: None,
        credentials: None,
        api_base: Some("https://settings.example".to_owned()),
        deployments: Vec::new(),
    };
    let defaults = Defaults {
        vertex: &vertex,
        embedding: &embedding,
    };
    let arms = [
        prepare(
            request(
                EMBED_CONTENT_MODEL,
                EmbeddingInput::Text("hello".to_owned()),
                "{}",
            ),
            defaults,
            &embed_content_catalog(),
        )
        .expect("a prepared request"),
        prepare(
            request(
                TEXT_MODEL,
                EmbeddingInput::Text("hello".to_owned()),
                r#"{"image": null}"#,
            ),
            defaults,
            &ModelCatalog::default(),
        )
        .expect("a prepared request"),
        prepare(
            request(TEXT_MODEL, EmbeddingInput::Text("hello".to_owned()), "{}"),
            defaults,
            &ModelCatalog::default(),
        )
        .expect("a prepared request"),
    ];
    let resolved: Vec<&Resolution> = arms.iter().map(|arm| labelled(arm).1).collect();
    assert_eq!(
        resolved
            .iter()
            .map(|arm| arm.project.as_deref())
            .collect::<Vec<_>>(),
        [Some("settings-project"); 3]
    );
    assert_eq!(
        resolved
            .iter()
            .map(|arm| arm.location.as_deref())
            .collect::<Vec<_>>(),
        [Some("settings-location"); 3]
    );
    assert_eq!(
        resolved
            .iter()
            .map(|arm| arm
                .credentials
                .as_ref()
                .and_then(ConfiguredCredentials::as_text))
            .collect::<Vec<_>>(),
        [Some("settings-key"); 3]
    );
    // the explicit api_base wins over the setting, as in main.py's or chain
    assert_eq!(
        resolved
            .iter()
            .map(|arm| arm.api_base.as_deref())
            .collect::<Vec<_>>(),
        [Some("https://vertex.example"); 3]
    );
    assert!(matches!(arms[0], VertexEmbeddingPrepared::EmbedContent(_)));
    assert!(matches!(arms[1], VertexEmbeddingPrepared::Multimodal(_)));
    assert!(matches!(arms[2], VertexEmbeddingPrepared::Text(_)));
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
    assert_multimodal_arguments();
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
    async fn embed_content(&self, call: &EmbedContentArguments) -> EmbedContentOutcome {
        self.record("embed_content", &call.model);
        Ok(reply())
    }

    async fn multimodal(&self, call: &MultimodalArguments) -> MultimodalOutcome {
        self.record("multimodal", &call.model);
        Ok(answer())
    }

    async fn text(&self, call: &TextArguments) -> TextOutcome {
        self.record("text", &call.model);
        Ok(response())
    }
}

#[tokio::test]
async fn runs_each_arm_through_its_handler() {
    let recorder = Recorder::default();
    let arms = [
        prepared(
            EMBED_CONTENT_MODEL,
            EmbeddingInput::Text("hello".to_owned()),
            "{}",
            &embed_content_catalog(),
        ),
        prepared(
            TEXT_MODEL,
            EmbeddingInput::Text("hello".to_owned()),
            r#"{"video": null}"#,
            &ModelCatalog::default(),
        ),
        prepared(
            TEXT_MODEL,
            EmbeddingInput::Tokens(vec![1, 2]),
            "{}",
            &ModelCatalog::default(),
        ),
    ];
    let results = [
        execute(&arms[0], &recorder)
            .await
            .expect("a handler result"),
        execute(&arms[1], &recorder)
            .await
            .expect("a handler result"),
        execute(&arms[2], &recorder)
            .await
            .expect("a handler result"),
    ];
    assert!(matches!(results[0], VertexEmbeddingResult::EmbedContent(_)));
    assert!(matches!(results[1], VertexEmbeddingResult::Multimodal(_)));
    assert!(matches!(results[2], VertexEmbeddingResult::Text(_)));
    // each handler receives the prepared call of its own arm
    assert_eq!(
        recorder.calls(),
        [
            ("embed_content", EMBED_CONTENT_MODEL.to_owned()),
            ("multimodal", TEXT_MODEL.to_owned()),
            ("text", TEXT_MODEL.to_owned()),
        ]
    );
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
    .err()
    .expect("tokens have no embedContent form");
    assert!(matches!(error, VertexEmbeddingPrepareError::TokenInput));
}

struct Failing;

impl VertexEmbeddingHandlers for Failing {
    async fn embed_content(&self, _call: &EmbedContentArguments) -> EmbedContentOutcome {
        Err(GeminiEmbeddingCallError::MissingApiKey)
    }

    async fn multimodal(&self, _call: &MultimodalArguments) -> MultimodalOutcome {
        Err(VertexEmbeddingCallError::Timeout)
    }

    async fn text(&self, _call: &TextArguments) -> TextOutcome {
        Err(VertexEmbeddingCallError::Status {
            status: 429,
            body: br#"{"error": {"message": "quota"}}"#.to_vec(),
        })
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
    let error = execute(&text, &Failing)
        .await
        .err()
        .expect("a handler error");
    let VertexEmbeddingError::Prediction(VertexEmbeddingCallError::Status { status, .. }) = error
    else {
        panic!("the text handler's status error travels unchanged");
    };
    assert_eq!(status, 429);
}
