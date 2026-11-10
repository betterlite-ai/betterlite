#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::grounding::log_grounding_spans;
use super::support::child;
use super::support::{generation, prompt_value, provider, sdk_tracer, spans};

fn enrichments(text: &str) -> IndexMap<String, Json> {
    match prompt_value(text) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("an object fixture")
        }
    }
}

#[rstest]
#[case::no_enrichments("{}")]
#[case::a_null_hidden_params(r#"{"hidden_params": null}"#)]
#[case::non_mapping_hidden_params(r#"{"hidden_params": "not-a-mapping"}"#)]
#[case::empty_hidden_params(r#"{"hidden_params": {}}"#)]
#[case::a_null_grounding_value(r#"{"hidden_params": {"vertex_ai_grounding_metadata": null}}"#)]
fn grounding_hidden_without_provider_metadata_logs_no_span(#[case] text: &str) {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    log_grounding_spans(&observations, &enrichments(text));
    assert!(spans(&exporter).is_empty());
}
