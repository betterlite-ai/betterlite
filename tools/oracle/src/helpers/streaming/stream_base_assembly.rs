use betterlite_streaming::stream_base_assembly::records::Chunk;
use betterlite_streaming::stream_chunk_shaping::records::JsonObject;
use serde::Deserialize;
use serde_json::{Value, json};

mod fixtures;
#[path = "stream_base_assembly/tests.rs"]
mod tests;
mod views;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Batch {
        cases: Vec<Self>,
    },
    ConditionalSortOriginalFirst {
        chunks: Vec<Chunk>,
    },
    BaseMetadata {
        chunks: Vec<Chunk>,
    },
    FinishAndHidden {
        chunks: Vec<Chunk>,
    },
    UpdateHiddenParams {
        #[serde(default)]
        response: Box<ResponseSpec>,
        #[serde(default)]
        chunk: Option<Box<Chunk>>,
    },
    AssembledProviderMetadata {
        cases: Vec<ProviderScenario>,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ResponseSpec {
    model: String,
    hidden_params: JsonObject,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderScenario {
    #[serde(default)]
    model: String,
    #[serde(default)]
    chunks: Vec<Chunk>,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    hidden_params: JsonObject,
    #[serde(default)]
    selector: SelectorFixture,
    #[serde(default)]
    projection: ProjectionFixture,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SelectorFixture {
    #[default]
    Table,
    None,
    Raise,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProjectionFixture {
    #[default]
    Record,
    Raise,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Batch { cases } => {
                let results: Result<Vec<Value>, _> = cases.iter().map(Self::run).collect();
                Ok(json!(results?))
            }
            Input::ConditionalSortOriginalFirst { chunks } => Ok(views::sort_view(chunks)),
            Input::BaseMetadata { chunks } | Input::FinishAndHidden { chunks } => {
                Ok(views::response_json(chunks))
            }
            Input::UpdateHiddenParams { response, chunk } => {
                Ok(views::update_json(response, chunk.as_deref()))
            }
            Input::AssembledProviderMetadata { cases } => fixtures::assembled_json(cases),
        }
    }
}
