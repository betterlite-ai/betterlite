use betterlite_models::decisions::Json;
use betterlite_vertex::gemini_metadata::annotations::grounding_annotations;
use betterlite_vertex::gemini_metadata::extension::{MetadataExtension, MetadataField};
use betterlite_vertex::gemini_metadata::extract::extract_candidate_metadata;
use betterlite_vertex::gemini_metadata::stream::assemble_stream_metadata;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Call {
    ScalarVsList,
    NestedSafety,
    HiddenFallback,
    EmptyVisibleShadow,
    DuplicateChunks,
    FirstNonwebIndex,
    CitationEmptyTitle,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    call: Call,
    #[serde(default)]
    candidates: Vec<Json>,
    #[serde(default)]
    chunks: Vec<Json>,
    #[serde(default)]
    grounding_metadata: Vec<Json>,
    // the Python side passes it to the pinned conversion, which ignores it
    #[serde(default, rename = "content_text")]
    _content_text: Option<String>,
}

pub(super) fn run(input: &Input) -> Value {
    match input.call {
        Call::ScalarVsList => scalar_vs_list(&input.candidates),
        Call::NestedSafety => nested_safety(&input.candidates),
        Call::HiddenFallback | Call::EmptyVisibleShadow | Call::DuplicateChunks => {
            assembled(&input.chunks)
        }
        Call::FirstNonwebIndex | Call::CitationEmptyTitle => {
            json!({"annotations": grounding_annotations(&input.grounding_metadata)})
        }
    }
}

fn scalar_vs_list(candidates: &[Json]) -> Value {
    let extracted: Vec<Value> = candidates
        .iter()
        .map(|candidate| {
            let extension = extract_candidate_metadata(candidate);
            json!({
                "grounding_metadata": extension.grounding_metadata,
                "url_context_metadata": extension.url_context_metadata,
                "safety_ratings": extension.safety_ratings,
                "citation_metadata": extension.citation_metadata,
            })
        })
        .collect();
    json!({"candidates": extracted})
}

fn nested_safety(candidates: &[Json]) -> Value {
    let applied: Vec<Value> = candidates
        .iter()
        .map(|candidate| {
            let extension = extract_candidate_metadata(candidate);
            json!({
                "fields": all_fields(&extension),
                "hidden": object(&extension.populated_fields()),
            })
        })
        .collect();
    json!({"candidates": applied})
}

fn all_fields(extension: &MetadataExtension) -> Value {
    let fields: Vec<(&'static str, &[Json])> = MetadataField::ALL
        .into_iter()
        .map(|field| (field.wire_name(), extension.values(field)))
        .collect();
    object(&fields)
}

fn assembled(chunks: &[Json]) -> Value {
    let extension = assemble_stream_metadata(chunks);
    let populated = object(&extension.populated_fields());
    json!({"fields": populated, "hidden": populated})
}

fn object(fields: &[(&str, &[Json])]) -> Value {
    let mut entries = serde_json::Map::new();
    for (name, values) in fields {
        entries.insert((*name).to_owned(), json!(values));
    }
    Value::Object(entries)
}
