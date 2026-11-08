use betterlite_streaming::stream_chunk_shaping::records::{
    ChunkResponseObject, GenericChunk, JsonObject, JsonValue, StreamChunk, StreamDelta,
    StreamOptions,
};
use betterlite_streaming::stream_chunk_shaping::usage::{TokenDetails, Usage};
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperResult};

mod steps;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    pub(crate) state: StateInput,
    pub(crate) repeated_chunk_limit: usize,
    #[serde(default)]
    pub(crate) usage_cost_providers: Vec<String>,
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct StateInput {
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) provider: Option<String>,
    #[serde(default)]
    pub(crate) merge_reasoning_in_choices: Option<bool>,
    #[serde(default)]
    pub(crate) base_hidden_params: JsonObject,
    #[serde(default)]
    pub(crate) response_id: Option<String>,
    #[serde(default)]
    pub(crate) created: Option<i64>,
    #[serde(default)]
    pub(crate) system_fingerprint: Option<String>,
    #[serde(default)]
    pub(crate) sent_first_chunk: Option<bool>,
    #[serde(default)]
    pub(crate) sent_last_chunk: Option<bool>,
    #[serde(default)]
    pub(crate) sent_first_thinking_block: Option<bool>,
    #[serde(default)]
    pub(crate) sent_last_thinking_block: Option<bool>,
    #[serde(default)]
    pub(crate) holding_chunk: String,
    #[serde(default)]
    pub(crate) response_uptil_now: String,
    #[serde(default)]
    pub(crate) received_finish_reason: Option<String>,
    #[serde(default)]
    pub(crate) intermittent_finish_reason: Option<String>,
    #[serde(default)]
    pub(crate) tool_call: Option<bool>,
    #[serde(default)]
    pub(crate) chunks: Vec<StreamChunk>,
    pub(crate) repeated_messages_count: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Creator {
        #[serde(default)]
        chunk: Option<JsonObject>,
        #[serde(default)]
        hidden_params: Option<JsonObject>,
    },
    SetModelId {
        id: String,
        chunk: StreamChunk,
    },
    CopyProviderFields {
        original: StreamChunk,
        chunk: StreamChunk,
    },
    IsChunkNonEmpty {
        completion: StreamDelta,
        chunk: StreamChunk,
        #[serde(default)]
        response: ChunkResponseObject,
    },
    Returns {
        completion: StreamDelta,
        chunk: StreamChunk,
        #[serde(default)]
        response: ChunkResponseObject,
    },
    CombineThinking {
        chunk: StreamChunk,
    },
    SpecialTokens {
        chunk: String,
        #[serde(default)]
        finish_reason: Option<String>,
    },
    Repetition,
    FinishReason,
    RecordUsage {
        chunk: StreamChunk,
    },
    StripSse {
        #[serde(default)]
        chunk: Option<String>,
    },
    GenericKnown {
        chunk: JsonObject,
    },
    ConvertGeneric {
        chunk: GenericChunk,
    },
    TotalUsage,
    BackfillUsage {
        usage: Usage,
    },
    ResolveCost {
        cost: JsonValue,
    },
    PropagateCost {
        chunk: StreamChunk,
    },
    CacheTokenCount {
        #[serde(default)]
        details: Option<TokenDetails>,
        keys: Vec<String>,
    },
    CoerceDetails {
        details: TokenDetails,
    },
    ProviderResponseModel {
        chunk: StreamChunk,
    },
    ProviderHiddenParams {
        chunk: StreamChunk,
        #[serde(default)]
        provider_response_model: Option<String>,
    },
    StampServiceTier {
        chunk: StreamChunk,
        complete: StreamChunk,
    },
    CheckStreamUsage {
        #[serde(default)]
        stream_options: Option<StreamOptions>,
    },
    CheckFunctionCall {
        optional_params: JsonObject,
    },
    MappedFinishReason {
        raw: String,
    },
    SetState {
        patch: StatePatch,
    },
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct StatePatch {
    #[serde(default)]
    pub(crate) sent_first_chunk: Option<bool>,
    #[serde(default)]
    pub(crate) sent_last_chunk: Option<bool>,
    #[serde(default)]
    pub(crate) holding_chunk: Option<String>,
    #[serde(default)]
    pub(crate) received_finish_reason: Option<String>,
    #[serde(default)]
    pub(crate) intermittent_finish_reason: Option<String>,
    #[serde(default)]
    pub(crate) tool_call: Option<bool>,
    #[serde(default)]
    pub(crate) sent_first_thinking_block: Option<bool>,
    #[serde(default)]
    pub(crate) sent_last_thinking_block: Option<bool>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        steps::run(input).map(|outputs| json!(outputs))
    }
}
