use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_foundry::embeddings::request::FoundryEmbeddingOptions;
use betterlite_http::client_kind::ClientKind;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingVector};
use betterlite_prompts::arguments::json::Object as PromptObject;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use super::support::{self, Env, KEY, field};
use crate::foundry_dispatch::credentials;
use crate::foundry_dispatch::embedding;
use crate::foundry_dispatch::fixtures::embedding::{Factory, ImageSent, Text, TextSent};

// One text fixture and one image fixture behind a two-item input, built fresh for each step
struct Rig {
    text: Text,
    factory: Factory,
    input: [String; 2],
    params: PromptObject,
}

impl Rig {
    fn new() -> Self {
        Self {
            text: Text {
                data: vec![EmbeddingRecord {
                    object: String::from("embedding"),
                    embedding: EmbeddingVector::Floats(vec![Number::from(1_u32)]),
                    index: 0,
                }],
                model: Some(String::from("text-embedding")),
                sent: Mutex::new(Vec::new()),
            },
            factory: Factory {
                status: 200,
                image_body: String::from(
                    r#"{"object":"list","data":[{"object":"embedding","embedding":[0.25],"index":0}],"model":"cohere-embed"}"#,
                ),
                sent: Arc::new(Mutex::new(Vec::new())),
            },
            input: [
                String::from("data:image/png;base64,aGVsbG8="),
                String::from("plain text"),
            ],
            params: PromptObject::new(),
        }
    }

    fn call<'rig>(
        &'rig self,
        api_base: Option<&'rig str>,
        api_key: Option<&'rig SecretString>,
    ) -> embedding::Call<'rig> {
        embedding::Call {
            model: "text-embedding",
            input: &self.input,
            options: FoundryEmbeddingOptions::default(),
            api_base,
            api_key,
            params: &self.params,
            timeout: Duration::from_secs(5),
            mode: ClientKind::Asynchronous,
        }
    }

    // Runs one dispatch and returns how many records and which model the response carries
    fn run(&self, call: &embedding::Call<'_>, env: &Env) -> (usize, Option<String>) {
        let response = support::block_on(embedding::dispatch(
            call,
            &env.globals(),
            None,
            &self.factory,
            &self.text,
        ))
        .expect("the embedding dispatch runs");
        (response.data.len(), response.model)
    }

    fn images(&self) -> Vec<ImageSent> {
        self.factory
            .sent
            .lock()
            .expect("the image requests")
            .to_vec()
    }

    fn texts(&self) -> Vec<TextSent> {
        self.text.snapshot().to_vec()
    }
}

fn environment() -> Env {
    Env::new()
        .environment_base("https://res.services.ai.azure.com")
        .environment_key(KEY)
}

pub(super) fn embedding_case() {
    image_and_text_halves_share_base_and_key();
    explicit_key_beats_globals();
    explicit_base_beats_environment();
    entra_token_fills_key_slot();
}

// the image half runs through the image transport with the resolved base and key, the text half gets the Foundry route base
fn image_and_text_halves_share_base_and_key() {
    let rig = Rig::new();
    let (records, model) = rig.run(&rig.call(None, None), &environment());
    assert_eq!(records, 2);
    assert_eq!(model.as_deref(), Some("text-embedding"));

    let images = rig.images();
    assert_eq!(images.len(), 1);
    assert_eq!(
        images[0].url,
        "https://res.services.ai.azure.com/models/images/embeddings"
    );
    let bearer = format!("Bearer {KEY}");
    assert!(
        images[0]
            .headers
            .iter()
            .any(|(name, value)| name == "authorization" && value == &bearer)
    );
    let image_body: JsonValue = serde_json::from_str(&images[0].body).expect("the image body");
    let JsonValue::Array(image_input) = field(&image_body, "input") else {
        panic!("the image input is an array");
    };
    assert_eq!(
        field(image_input.first().expect("one image"), "image"),
        &JsonValue::Text(String::from("data:image/png;base64,aGVsbG8="))
    );

    let texts = rig.texts();
    assert_eq!(texts.len(), 1);
    assert_eq!(
        texts[0].api_base.as_deref(),
        Some("https://res.services.ai.azure.com/models")
    );
    assert_eq!(texts[0].api_key.as_deref(), Some(KEY));
}

// an explicit key beats the global and environment values
fn explicit_key_beats_globals() {
    let rig = Rig::new();
    let explicit = SecretString::from("explicit-key");
    let env = Env::new().global_key("global-key").environment_key(KEY);
    rig.run(&rig.call(None, Some(&explicit)), &env);
    assert_eq!(rig.texts()[0].api_key.as_deref(), Some("explicit-key"));
}

// an explicit base beats the environment value for the text half
fn explicit_base_beats_environment() {
    let rig = Rig::new();
    let call = rig.call(Some("https://explicit.services.ai.azure.com"), None);
    rig.run(&call, &environment());
    assert_eq!(
        rig.texts()[0].api_base.as_deref(),
        Some("https://explicit.services.ai.azure.com/models")
    );
}

// an azure_ad_token parameter fills the key slot once every key string is unset
fn entra_token_fills_key_slot() {
    let empty = Env::new();
    let entra_params: PromptObject = serde_json::from_str(r#"{"azure_ad_token":"entra-token"}"#)
        .expect("the entra token parameter");
    let token = credentials::resolve_embedding_key(None, &entra_params, &empty.globals())
        .expect("the entra token resolves")
        .expect("the entra token fills the key slot");
    assert_eq!(token.expose_secret(), "entra-token");
}
