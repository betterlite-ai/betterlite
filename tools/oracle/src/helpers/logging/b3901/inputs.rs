use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    ResolveTraceId {
        input: Option<String>,
    },
    ResolveObservationId {
        input: Option<String>,
    },
    ToUnixNanos {
        seconds: Option<Decimal>,
        datetime: Option<String>,
    },
    TraceAttributes {
        fields: TraceFields,
    },
    ObservationAttributes {
        fields: ObservationFields,
    },
    Sampler {
        rate: f64,
        trace_id: String,
        attributes: Vec<(String, Json)>,
    },
    ParentContext {
        trace_id: String,
        parent_observation_id: Option<String>,
        existing_trace: bool,
        fallback_span_id: String,
    },
    StartChildSpan {
        parent_trace_id: String,
        parent_span_id: String,
        parent_public: Option<bool>,
        name: String,
        seconds: Option<Decimal>,
        datetime: Option<String>,
        attributes: Vec<(String, Json)>,
        child_span_id: String,
    },
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TraceFields {
    pub(crate) name: Option<Json>,
    pub(crate) user_id: Option<Json>,
    pub(crate) session_id: Option<Json>,
    pub(crate) version: Option<Json>,
    pub(crate) release: Option<Json>,
    pub(crate) tags: Option<Json>,
    pub(crate) metadata: Option<Json>,
    pub(crate) public: Option<bool>,
    pub(crate) input: Option<Json>,
    pub(crate) output: Option<Json>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObservationFields {
    pub(crate) observation_type: String,
    pub(crate) input: Option<Json>,
    pub(crate) output: Option<Json>,
    pub(crate) metadata: Option<Json>,
    pub(crate) level: Option<Json>,
    pub(crate) status_message: Option<Json>,
    pub(crate) version: Option<Json>,
    pub(crate) model: Option<Json>,
    pub(crate) model_parameters: Option<Json>,
    pub(crate) usage_details: Option<Json>,
    pub(crate) cost_details: Option<Json>,
    pub(crate) completion_start_time: Option<String>,
    pub(crate) prompt: Option<PromptFields>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PromptFields {
    pub(crate) name: String,
    pub(crate) version: Option<i64>,
    pub(crate) is_fallback: bool,
}
