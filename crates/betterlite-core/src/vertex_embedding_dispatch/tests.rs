#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::{
    ConfiguredCredentials, CredentialSource,
};
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_config::sections::vertex_embedding_dispatch::Settings as EmbeddingSettings;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::{Extra, Json};
use rstest::rstest;
use secrecy::SecretString;
use tracing::subscriber::{DefaultGuard, set_default};

use super::{Defaults, MULTIMODAL_EMBEDDING_MODELS, Native, SelectionRequest, pin, select};

const MODEL: &str = "text-embedding-005";
const EMBED_CONTENT_MODEL: &str = "gemini-embedding-2";
const ACCOUNT: &str = r#"{"type": "service_account", "client_email": "sa@example.iam", "private_key": "key", "project_id": "object-project"}"#;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        let bytes = self.0.lock().expect("the capture lock").clone();
        String::from_utf8(bytes).expect("the captured log is UTF-8")
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

fn catalog(source: &str) -> ModelCatalog {
    ModelCatalog::parse(source.as_bytes(), UnknownFields::Reject).expect("a catalog")
}

fn params(source: &str) -> Extra {
    serde_json::from_str(source).expect("optional params")
}

fn credentials(text: &str) -> ConfiguredCredentials {
    ConfiguredCredentials::Text(SecretString::from(text.to_owned()))
}

fn resolution(
    model: &str,
    optional_params: &Extra,
    vertex: &VertexSettings,
    embedding: &EmbeddingSettings,
    catalog: &ModelCatalog,
) -> super::Resolution {
    select(
        SelectionRequest {
            model,
            api_base: None,
            optional_params,
            pinned: None,
        },
        Defaults { vertex, embedding },
        catalog,
    )
    .expect("a resolved request")
}

// the two settings snapshots a real config load hands the dispatch
fn loaded_resolution(source: &str, environment: &[(&str, &str)]) -> super::Resolution {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let loaded = ConfigLoader { lookup }
        .load(source)
        .expect("a loaded config");
    let empty = params("{}");
    select(
        SelectionRequest {
            model: MODEL,
            api_base: None,
            optional_params: &empty,
            pinned: None,
        },
        Defaults {
            vertex: &loaded.sections.vertex_credential_sources,
            embedding: &loaded.sections.vertex_embedding_dispatch,
        },
        &ModelCatalog::default(),
    )
    .expect("a resolved request")
}

#[test]
fn catalog_before_media() {
    let catalog = catalog(&format!(
        r#"{{"vertex_ai/{EMBED_CONTENT_MODEL}": {{"litellm_provider": "vertex_ai", "mode": "embedding", "uses_embed_content": true}}}}"#
    ));
    let optional_params =
        params(r#"{"image": "gs://bucket/cat.png", "video": "gs://bucket/dog.mp4"}"#);
    let vertex = VertexSettings::default();
    let embedding = EmbeddingSettings::default();
    let resolved = resolution(
        EMBED_CONTENT_MODEL,
        &optional_params,
        &vertex,
        &embedding,
        &catalog,
    );
    assert_eq!(resolved.native, Native::EmbedContent);
}

#[rstest]
#[case::image(r#"{"image": null}"#)]
#[case::video(r#"{"video": null}"#)]
#[case::both(r#"{"image": null, "video": null}"#)]
fn null_image_selects_media(#[case] source: &str) {
    let catalog = ModelCatalog::default();
    let vertex = VertexSettings::default();
    let embedding = EmbeddingSettings::default();
    let optional_params = params(source);
    let resolved = resolution(MODEL, &optional_params, &vertex, &embedding, &catalog);
    assert_eq!(resolved.native, Native::Multimodal);
    // the media keys stay in the params the multimodal handler reads
    assert_eq!(resolved.optional_params, optional_params);
}

#[test]
fn absent_media_selects_text() {
    let catalog = ModelCatalog::default();
    let vertex = VertexSettings::default();
    let embedding = EmbeddingSettings::default();
    let optional_params = params("{}");
    let resolved = resolution(MODEL, &optional_params, &vertex, &embedding, &catalog);
    assert_eq!(resolved.native, Native::Text);
}

#[test]
fn unknown_model_text_fallback() {
    let catalog = catalog(
        r#"{"vertex_ai/other-embedding": {"litellm_provider": "vertex_ai", "mode": "embedding"}}"#,
    );
    let optional_params = params(r#"{"dimensions": 3}"#);
    let vertex = VertexSettings::default();
    let embedding = EmbeddingSettings::default();
    let (captured, _guard) = capture();
    let resolved = resolution(MODEL, &optional_params, &vertex, &embedding, &catalog);
    assert_eq!(resolved.native, Native::Text);
    // main.py's except block is silent, so the fallback adds no warning either
    assert!(!captured.text().contains("WARN"), "{}", captured.text());
}

#[test]
fn alias_pop_short_circuit() {
    let catalog = ModelCatalog::default();
    let vertex = VertexSettings::default();
    let embedding = EmbeddingSettings::default();
    let both = params(r#"{"vertex_project": "listed", "vertex_ai_project": "second"}"#);
    let resolved = resolution(MODEL, &both, &vertex, &embedding, &catalog);
    assert_eq!(resolved.project.as_deref(), Some("listed"));
    // the first truthy alias stops the chain before the second pop
    assert!(!resolved.optional_params.contains_key("vertex_project"));
    assert_eq!(
        resolved.optional_params.get("vertex_ai_project"),
        Some(&Json::Text("second".to_owned()))
    );

    let first_null = params(r#"{"vertex_project": null, "vertex_ai_project": "second"}"#);
    let nested = resolution(MODEL, &first_null, &vertex, &embedding, &catalog);
    assert_eq!(nested.project.as_deref(), Some("second"));
    assert!(nested.optional_params.is_empty());
}

#[test]
fn location_and_credential_alias_order_matches_the_source() {
    let catalog = ModelCatalog::default();
    let vertex = VertexSettings::default();
    let embedding = EmbeddingSettings::default();
    let both = params(
        r#"{"vertex_location": "first-location", "vertex_ai_location": "second-location", "vertex_credentials": "first-creds", "vertex_ai_credentials": "second-creds"}"#,
    );
    let resolved = resolution(MODEL, &both, &vertex, &embedding, &catalog);
    assert_eq!(resolved.location.as_deref(), Some("first-location"));
    assert_eq!(
        resolved
            .credentials
            .as_ref()
            .and_then(ConfiguredCredentials::as_text),
        Some("first-creds")
    );
    // each chain stops at its first alias, so the second one stays for the handler
    assert_eq!(
        resolved.optional_params.get("vertex_ai_location"),
        Some(&Json::Text("second-location".to_owned()))
    );
    assert_eq!(
        resolved.optional_params.get("vertex_ai_credentials"),
        Some(&Json::Text("second-creds".to_owned()))
    );

    let first_null = params(r#"{"vertex_location": null, "vertex_ai_location": "second"}"#);
    let nested = resolution(MODEL, &first_null, &vertex, &embedding, &catalog);
    assert_eq!(nested.location.as_deref(), Some("second"));
}

#[rstest]
#[case::multimodal("multimodalembedding", Native::Multimodal)]
#[case::multimodal_001("multimodalembedding@001", Native::Multimodal)]
// main.py compares the split model by name, so a near miss falls through to text
#[case::multimodal_002("multimodalembedding@002", Native::Text)]
#[case::capitalized("MultimodalEmbedding", Native::Text)]
fn multimodal_model_names_match_exactly(#[case] model: &str, #[case] native: Native) {
    let catalog = ModelCatalog::default();
    let optional_params = params("{}");
    let vertex = VertexSettings::default();
    let embedding = EmbeddingSettings::default();
    let resolved = resolution(model, &optional_params, &vertex, &embedding, &catalog);
    assert_eq!(resolved.native, native);
}

#[test]
fn the_multimodal_model_table_is_the_source_pair() {
    assert_eq!(
        MULTIMODAL_EMBEDDING_MODELS,
        ["multimodalembedding", "multimodalembedding@001"]
    );
}

#[test]
fn environment_aliases_fall_back_in_source_order() {
    let catalog = ModelCatalog::default();
    let optional_params = params("{}");
    let embedding = EmbeddingSettings {
        project: Some("vertex-project".to_owned()),
        credentials: Some(credentials("vertex-key")),
        api_base: Some("https://vertex.example".to_owned()),
    };
    let vertex = VertexSettings {
        project: Some("vertex-ai-project".to_owned()),
        location: Some("vertex-ai-location".to_owned()),
        credentials: Some(credentials("vertex-ai-key")),
        adc: AdcEnvironment::default(),
    };
    let resolved = resolution(MODEL, &optional_params, &vertex, &embedding, &catalog);
    assert_eq!(resolved.project.as_deref(), Some("vertex-ai-project"));
    assert_eq!(resolved.location.as_deref(), Some("vertex-ai-location"));
    assert!(matches!(
        resolved.credentials,
        Some(ConfiguredCredentials::Text(_))
    ));
    assert_eq!(resolved.api_base.as_deref(), Some("https://vertex.example"));
}

#[test]
fn settings_win_over_the_environment_through_the_loader() {
    let resolved = loaded_resolution(
        "litellm_settings:\n  vertex_project: settings-project\n  vertex_location: settings-location\n  api_base: settings-base\n",
        &[
            ("VERTEXAI_PROJECT", "ai-project"),
            ("VERTEX_PROJECT", "env-project"),
            ("VERTEXAI_LOCATION", "ai-location"),
            ("VERTEX_LOCATION", "env-location"),
            ("VERTEXAI_CREDENTIALS", "ai-creds"),
            ("VERTEX_CREDENTIALS", "env-creds"),
            ("VERTEXAI_API_BASE", "ai-base"),
            ("VERTEX_API_BASE", "env-base"),
        ],
    );
    assert_eq!(resolved.project.as_deref(), Some("settings-project"));
    assert_eq!(resolved.location.as_deref(), Some("settings-location"));
    assert_eq!(
        resolved
            .credentials
            .as_ref()
            .and_then(ConfiguredCredentials::as_text),
        Some("ai-creds")
    );
    assert_eq!(resolved.api_base.as_deref(), Some("settings-base"));
}

#[test]
fn environment_aliases_keep_the_loader_order() {
    let resolved = loaded_resolution(
        "",
        &[
            ("VERTEXAI_PROJECT", "ai-project"),
            ("VERTEX_PROJECT", "env-project"),
            ("VERTEXAI_LOCATION", "ai-location"),
            ("VERTEX_LOCATION", "env-location"),
            ("VERTEXAI_CREDENTIALS", "ai-creds"),
            ("VERTEX_CREDENTIALS", "env-creds"),
            ("VERTEXAI_API_BASE", "ai-base"),
            ("VERTEX_API_BASE", "env-base"),
        ],
    );
    assert_eq!(resolved.project.as_deref(), Some("ai-project"));
    assert_eq!(resolved.location.as_deref(), Some("ai-location"));
    assert_eq!(
        resolved
            .credentials
            .as_ref()
            .and_then(ConfiguredCredentials::as_text),
        Some("ai-creds")
    );
    assert_eq!(resolved.api_base.as_deref(), Some("ai-base"));
}

#[test]
fn the_section_input_is_the_last_fallback() {
    let catalog = ModelCatalog::default();
    let optional_params = params("{}");
    let embedding = EmbeddingSettings {
        project: Some("vertex-project".to_owned()),
        credentials: None,
        api_base: Some("https://vertex.example".to_owned()),
    };
    let vertex = VertexSettings::default();
    let resolved = resolution(MODEL, &optional_params, &vertex, &embedding, &catalog);
    assert_eq!(resolved.project.as_deref(), Some("vertex-project"));
    assert_eq!(resolved.api_base.as_deref(), Some("https://vertex.example"));
}

#[test]
fn an_explicit_api_base_and_option_credentials_win() {
    let catalog = ModelCatalog::default();
    let vertex = VertexSettings {
        project: None,
        location: Some("settings-location".to_owned()),
        credentials: None,
        adc: AdcEnvironment::default(),
    };
    let embedding = EmbeddingSettings {
        project: None,
        credentials: None,
        api_base: Some("https://settings.example".to_owned()),
    };
    let optional_params = params(
        r#"{"vertex_credentials": "{\"type\": \"service_account\"}", "vertex_location": "option-location"}"#,
    );
    let resolved = select(
        SelectionRequest {
            model: MODEL,
            api_base: Some("https://explicit.example"),
            optional_params: &optional_params,
            pinned: None,
        },
        Defaults {
            vertex: &vertex,
            embedding: &embedding,
        },
        &catalog,
    )
    .expect("a resolved request");
    assert_eq!(
        resolved.api_base.as_deref(),
        Some("https://explicit.example")
    );
    assert_eq!(resolved.location.as_deref(), Some("option-location"));
    assert!(matches!(
        resolved.credentials,
        Some(ConfiguredCredentials::Source(_))
    ));
    assert!(resolved.optional_params.is_empty());
}

#[test]
fn a_wrong_typed_alias_names_its_field() {
    let catalog = ModelCatalog::default();
    let vertex = VertexSettings::default();
    let embedding = EmbeddingSettings::default();
    let optional_params = params(r#"{"vertex_project": 5}"#);
    let error = select(
        SelectionRequest {
            model: MODEL,
            api_base: None,
            optional_params: &optional_params,
            pinned: None,
        },
        Defaults {
            vertex: &vertex,
            embedding: &embedding,
        },
        &catalog,
    )
    .err()
    .expect("a rejected option");
    assert_eq!(
        error.to_string(),
        "the vertex_project option must be a string"
    );
}

fn rejected_credentials(optional_params: &Extra) -> String {
    let vertex = VertexSettings::default();
    let embedding = EmbeddingSettings::default();
    select(
        SelectionRequest {
            model: MODEL,
            api_base: None,
            optional_params,
            pinned: None,
        },
        Defaults {
            vertex: &vertex,
            embedding: &embedding,
        },
        &ModelCatalog::default(),
    )
    .err()
    .expect("a rejected credential")
    .to_string()
}

#[rstest]
#[case::vertex_credentials("vertex_credentials")]
#[case::vertex_ai_credentials("vertex_ai_credentials")]
fn an_object_credential_decodes_to_a_source(#[case] alias: &str) {
    let optional_params = params(&format!(r#"{{"{alias}": {ACCOUNT}}}"#));
    let resolved = resolution(
        MODEL,
        &optional_params,
        &VertexSettings::default(),
        &EmbeddingSettings::default(),
        &ModelCatalog::default(),
    );
    let Some(ConfiguredCredentials::Source(CredentialSource::ServiceAccount(account))) =
        resolved.credentials
    else {
        panic!("an inline service-account object decodes to a credential source");
    };
    assert_eq!(account.client_email, "sa@example.iam");
    assert_eq!(account.project_id.as_deref(), Some("object-project"));
    assert!(resolved.optional_params.is_empty());
}

#[rstest]
#[case::boolean("true")]
#[case::number("5")]
#[case::list(r#"["a"]"#)]
fn a_credential_of_another_type_names_its_field(#[case] value: &str) {
    let optional_params = params(&format!(r#"{{"vertex_credentials": {value}}}"#));
    assert_eq!(
        rejected_credentials(&optional_params),
        "the vertex_credentials option must be a string or an object"
    );
}

#[test]
fn an_unreadable_credential_object_names_its_field() {
    let optional_params = params(r#"{"vertex_ai_credentials": {"type": "service_account"}}"#);
    assert_eq!(
        rejected_credentials(&optional_params),
        "the vertex_ai_credentials option is not a readable credential"
    );
}

fn pinned_resolution(pins: &str, options: &str, vertex: &VertexSettings) -> super::Resolution {
    let pinned = pin(&params(pins)).expect("pinned options");
    let optional_params = params(options);
    select(
        SelectionRequest {
            model: MODEL,
            api_base: None,
            optional_params: &optional_params,
            pinned: Some(&pinned),
        },
        Defaults {
            vertex,
            embedding: &EmbeddingSettings::default(),
        },
        &ModelCatalog::default(),
    )
    .expect("a resolved request")
}

#[test]
fn pinned_options_sit_between_the_request_and_the_settings() {
    let vertex = VertexSettings {
        project: Some("settings-project".to_owned()),
        location: Some("settings-location".to_owned()),
        credentials: None,
        adc: AdcEnvironment::default(),
    };
    let pins = format!(
        r#"{{"vertex_ai_project": "pinned-project", "vertex_location": "pinned-location", "vertex_credentials": {ACCOUNT}, "api_base": "https://pinned.example"}}"#
    );
    let pinned_only = pinned_resolution(&pins, "{}", &vertex);
    assert_eq!(pinned_only.project.as_deref(), Some("pinned-project"));
    assert_eq!(pinned_only.location.as_deref(), Some("pinned-location"));
    assert_eq!(
        pinned_only.api_base.as_deref(),
        Some("https://pinned.example")
    );
    assert!(matches!(
        pinned_only.credentials,
        Some(ConfiguredCredentials::Source(_))
    ));
    let overridden = pinned_resolution(&pins, r#"{"vertex_project": "option-project"}"#, &vertex);
    assert_eq!(overridden.project.as_deref(), Some("option-project"));
    assert_eq!(overridden.location.as_deref(), Some("pinned-location"));
    let unpinned = pinned_resolution("{}", "{}", &vertex);
    assert_eq!(unpinned.project.as_deref(), Some("settings-project"));
    assert!(unpinned.credentials.is_none());
}

#[rstest]
#[case::project(
    r#"{"vertex_project": 5}"#,
    "the vertex_project option must be a string"
)]
#[case::api_base(r#"{"api_base": ["x"]}"#, "the api_base option must be a string")]
#[case::credentials(
    r#"{"vertex_credentials": true}"#,
    "the vertex_credentials option must be a string or an object"
)]
fn a_wrong_typed_pin_names_its_field(#[case] pins: &str, #[case] message: &str) {
    let error = pin(&params(pins)).expect_err("a rejected pin");
    assert_eq!(error.to_string(), message);
}
