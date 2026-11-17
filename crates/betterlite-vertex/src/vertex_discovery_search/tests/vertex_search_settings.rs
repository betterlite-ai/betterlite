use crate::vertex_discovery_search::options::{
    DEFAULT_COLLECTION_ID, RawSearchOptions, VertexSearchOptions,
};
use serde::Deserialize;

use betterlite_config::loader::{ConfigError, parse_view};

#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawSearchOptions,
}

// the loader's own view parses the keys until the section registry carries an entry for them
fn load(source: &str) -> Result<VertexSearchOptions, ConfigError> {
    let (raw, _unknown) = parse_view::<Raw>(source)?;
    Ok(VertexSearchOptions::from(
        raw.model_list
            .into_iter()
            .next()
            .map(|deployment| deployment.litellm_params)
            .unwrap_or_default(),
    ))
}

fn deployment(keys: &str) -> String {
    format!("model_list:\n  - litellm_params:\n      model: vertex_ai/search_api\n{keys}")
}

#[test]
fn loads_vertex_collection_id() {
    let configured = load(&deployment("      vertex_collection_id: team_collection\n"))
        .expect("the loader's view parses the key");
    assert_eq!(configured.vertex_collection_id, "team_collection");
    let absent = load(&deployment("")).expect("a config without the key loads");
    assert_eq!(absent.vertex_collection_id, DEFAULT_COLLECTION_ID);
    let nulled = load(&deployment("      vertex_collection_id: null\n"))
        .expect("a null key takes the default");
    assert_eq!(nulled.vertex_collection_id, DEFAULT_COLLECTION_ID);
}

#[test]
fn loads_vertex_engine_id() {
    let configured = load(&deployment("      vertex_engine_id: app-example\n"))
        .expect("the loader's view parses the key");
    assert_eq!(configured.vertex_engine_id.as_deref(), Some("app-example"));
    let absent = load(&deployment("")).expect("a config without the key loads");
    assert_eq!(absent.vertex_engine_id, None);
    let nulled =
        load(&deployment("      vertex_engine_id: null\n")).expect("a null key stays absent");
    assert_eq!(nulled.vertex_engine_id, None);
}

#[test]
fn loads_vertex_search_store_id() {
    let configured = load(&deployment("      vector_store_id: store-example\n"))
        .expect("the loader's view parses the key");
    assert_eq!(configured.vector_store_id.as_deref(), Some("store-example"));
    let absent = load(&deployment("")).expect("a config without the key loads");
    assert_eq!(absent.vector_store_id, None);
    let nulled =
        load(&deployment("      vector_store_id: null\n")).expect("a null key stays absent");
    assert_eq!(nulled.vector_store_id, None);
}

#[test]
fn a_non_text_target_field_fails_the_load() {
    let error = load(&deployment("      vertex_engine_id: [app]\n"))
        .expect_err("a list is not an engine id");
    assert!(matches!(error, ConfigError::Yaml { .. }), "{error}");
}
