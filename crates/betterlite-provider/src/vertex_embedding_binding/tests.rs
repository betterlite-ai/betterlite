#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_embedding_dispatch::Settings as EmbeddingSettings;
use betterlite_core::vertex_embedding_dispatch::Defaults;
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
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};

use super::arguments::{EmbedContentArguments, MultimodalArguments, TextArguments};
use super::{
    VertexEmbeddingError, VertexEmbeddingHandlers, VertexEmbeddingPrepareError,
    VertexEmbeddingPrepared, VertexEmbeddingRequest, VertexEmbeddingResult, execute, prepare,
};

const TEXT_MODEL: &str = "text-embedding-005";
const EMBED_CONTENT_MODEL: &str = "gemini-embedding-2";

fn catalog(source: &str) -> ModelCatalog {
    ModelCatalog::parse(source.as_bytes(), UnknownFields::Reject).expect("a catalog")
}

fn params(source: &str) -> Extra {
    serde_json::from_str(source).expect("optional params")
}

fn request(model: &str, input: EmbeddingInput, optional_params: &str) -> VertexEmbeddingRequest {
    VertexEmbeddingRequest {
        model: model.to_owned(),
        input,
        optional_params: params(optional_params),
        litellm_params: Some(params(r#"{"custom_llm_provider": "vertex_ai"}"#)),
        api_key: Some(SecretString::from("gemini-key".to_owned())),
        api_base: Some("https://vertex.example".to_owned()),
        extra_headers: BTreeMap::from([("x-goog-request-params".to_owned(), "a=b".to_owned())]),
        timeout: Some(Duration::from_secs(42)),
        aembedding: false,
    }
}

fn prepared(
    model: &str,
    input: EmbeddingInput,
    optional_params: &str,
    catalog: &ModelCatalog,
) -> VertexEmbeddingPrepared {
    let vertex = VertexSettings::default();
    let embedding = EmbeddingSettings::default();
    prepare(
        request(model, input, optional_params),
        Defaults {
            vertex: &vertex,
            embedding: &embedding,
        },
        catalog,
    )
    .expect("a prepared request")
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
        Some(Json::Object(params(
            r#"{"custom_llm_provider": "vertex_ai"}"#
        )))
    );
    assert_eq!(text.optional_params, Json::Object(Extra::new()));
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
    assert!(media.optional_params.contains_key("image"));
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
    assert_eq!(multimodal.optional_params.get("image"), Some(&Json::Null));
}

#[test]
fn branch_specific_arguments() {
    assert_text_arguments();
    assert_embed_content_arguments();
    assert_multimodal_arguments();
}

#[derive(Default)]
struct Recorder {
    called: Mutex<Vec<&'static str>>,
}

impl Recorder {
    fn record(&self, arm: &'static str) {
        self.called.lock().expect("the call log").push(arm);
    }

    fn calls(&self) -> Vec<&'static str> {
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
    fn embed_content<'call>(
        &'call self,
        _call: &'call EmbedContentArguments,
    ) -> impl Future<Output = Result<EmbeddingReply, GeminiEmbeddingCallError>> + Send + 'call {
        async move {
            self.record("embed_content");
            Ok(reply())
        }
    }

    fn multimodal<'call>(
        &'call self,
        _call: &'call MultimodalArguments,
    ) -> impl Future<Output = Result<MultimodalEmbeddingAnswer, VertexEmbeddingCallError>> + Send + 'call
    {
        async move {
            self.record("multimodal");
            Ok(answer())
        }
    }

    fn text<'call>(
        &'call self,
        _call: &'call TextArguments,
    ) -> impl Future<Output = Result<EmbeddingResponse, VertexEmbeddingCallError>> + Send + 'call
    {
        async move {
            self.record("text");
            Ok(response())
        }
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
    let mut results = Vec::new();
    for arm in &arms {
        results.push(execute(arm, &recorder).await.expect("a handler result"));
    }
    assert!(matches!(results[0], VertexEmbeddingResult::EmbedContent(_)));
    assert!(matches!(results[1], VertexEmbeddingResult::Multimodal(_)));
    assert!(matches!(results[2], VertexEmbeddingResult::Text(_)));
    assert_eq!(recorder.calls(), ["embed_content", "multimodal", "text"]);
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
    fn embed_content<'call>(
        &'call self,
        _call: &'call EmbedContentArguments,
    ) -> impl Future<Output = Result<EmbeddingReply, GeminiEmbeddingCallError>> + Send + 'call {
        async move { Err(GeminiEmbeddingCallError::MissingApiKey) }
    }

    fn multimodal<'call>(
        &'call self,
        _call: &'call MultimodalArguments,
    ) -> impl Future<Output = Result<MultimodalEmbeddingAnswer, VertexEmbeddingCallError>> + Send + 'call
    {
        async move { Err(VertexEmbeddingCallError::Timeout) }
    }

    fn text<'call>(
        &'call self,
        _call: &'call TextArguments,
    ) -> impl Future<Output = Result<EmbeddingResponse, VertexEmbeddingCallError>> + Send + 'call
    {
        async move {
            Err(VertexEmbeddingCallError::Status {
                status: 429,
                body: br#"{"error": {"message": "quota"}}"#.to_vec(),
            })
        }
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
