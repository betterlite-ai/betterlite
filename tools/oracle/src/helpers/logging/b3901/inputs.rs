use std::num::ParseIntError;

use betterlite_logging::langfuse_observations::attributes::{ObservationType, PromptLink};
use betterlite_logging::langfuse_observations::identity::Timestamp;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use opentelemetry::trace::{SpanId, TraceId};
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Deserialize)]
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
    ToUnixNanos(Moment),
    TraceAttributes {
        fields: TraceFields,
    },
    ObservationAttributes {
        fields: ObservationFields,
    },
    Sampler {
        rate: f64,
        trace_id: HexTraceId,
        attributes: Vec<(String, String)>,
    },
    ParentContext {
        trace_id: HexTraceId,
        parent_observation_id: Option<HexSpanId>,
        existing_trace: bool,
        fallback_span_id: HexSpanId,
    },
    StartChildSpan(ChildSpan),
}

#[derive(Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct HexTraceId(pub(crate) TraceId);

impl TryFrom<String> for HexTraceId {
    type Error = ParseIntError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        TraceId::from_hex(&text).map(Self)
    }
}

#[derive(Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct HexSpanId(pub(crate) SpanId);

impl TryFrom<String> for HexSpanId {
    type Error = ParseIntError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        SpanId::from_hex(&text).map(Self)
    }
}

#[derive(Deserialize)]
pub(crate) struct Moment {
    seconds: Option<Decimal>,
    datetime: Option<DateTime<Utc>>,
}

impl Moment {
    pub(crate) fn timestamp(&self) -> Option<Timestamp> {
        self.datetime
            .map(Timestamp::DateTime)
            .or_else(|| self.seconds.map(Timestamp::Seconds))
    }
}

#[derive(Deserialize)]
pub(crate) struct ChildSpan {
    pub(crate) parent_trace_id: HexTraceId,
    pub(crate) parent_span_id: HexSpanId,
    pub(crate) parent_public: Option<bool>,
    pub(crate) name: String,
    #[serde(flatten)]
    pub(crate) moment: Moment,
    pub(crate) attributes: Vec<(String, String)>,
    #[serde(rename = "child_span_id")]
    pub(crate) span_id: HexSpanId,
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
pub(crate) struct ObservationFields {
    pub(crate) observation_type: ObservationType,
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
    pub(crate) completion_start_time: Option<Timestamp>,
    pub(crate) prompt: Option<PromptLink>,
}
