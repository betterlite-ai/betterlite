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
        Call::ScalarVsList => rendered(&input.candidates, |extension| {
            json!({
                "grounding_metadata": extension.grounding_metadata,
                "url_context_metadata": extension.url_context_metadata,
                "safety_ratings": extension.safety_ratings,
                "citation_metadata": extension.citation_metadata,
            })
        }),
        Call::NestedSafety => rendered(&input.candidates, |extension| {
            json!({
                "fields": object(
                    &MetadataField::ALL
                        .map(|field| (field.wire_name(), extension.values(field)))
                ),
                "hidden": object(&extension.populated_fields()),
            })
        }),
        Call::HiddenFallback | Call::EmptyVisibleShadow | Call::DuplicateChunks => {
            let extension = assemble_stream_metadata(&input.chunks);
            let populated = object(&extension.populated_fields());
            json!({"fields": populated, "hidden": populated})
        }
        Call::FirstNonwebIndex | Call::CitationEmptyTitle => {
            json!({"annotations": grounding_annotations(&input.grounding_metadata)})
        }
    }
}

fn rendered(candidates: &[Json], render: fn(&MetadataExtension) -> Value) -> Value {
    let entries: Vec<Value> = candidates
        .iter()
        .map(|candidate| render(&extract_candidate_metadata(candidate)))
        .collect();
    json!({"candidates": entries})
}

fn object(fields: &[(&str, &[Json])]) -> Value {
    let mut entries = serde_json::Map::new();
    for (name, values) in fields {
        entries.insert((*name).to_owned(), json!(values));
    }
    Value::Object(entries)
}
