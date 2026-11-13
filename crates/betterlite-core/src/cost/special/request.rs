use std::collections::BTreeMap;

use betterlite_config::cost_adjustments::AdjustmentEntry;
use betterlite_models::billing_text::ResponseText;
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::protocol_cost::ProtocolCall;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::usage_sum::Usage;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::policy::SpecialPricePolicy;
use super::replicate::ReplicateStamps;
use crate::cost::context::{CostResponse, response_time::ResponseTiming};
use crate::cost::image_dispatch::options::SizePattern;
use crate::cost::image_dispatch::policy::ImageResponseView;
use crate::cost::prepare::{PreparedBillingAttempt, RerankUnits};
use crate::cost::realtime::Event;
use crate::routes::CallType;

// The usage record of a video response; the caller leaves it out when the response is a dictionary or the record is falsy.
#[derive(Debug, Clone, Default)]
pub struct VideoUsage<'usage> {
    pub duration_seconds: Option<Decimal>,
    pub video_resolution: Option<&'usage str>,
    pub provider_reported_cost_usd: Option<Decimal>,
    pub video_count: Option<i64>,
}

// _hidden_params.get("audio_transcription_duration", ...) distinguishes a missing key from an explicit null.
#[derive(Debug, Clone, Copy, Default)]
pub enum HiddenDuration {
    #[default]
    Missing,
    Null,
    Seconds(Decimal),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TranscriptionDuration {
    pub hidden: HiddenDuration,
    pub response_duration: Option<Decimal>,
}

#[derive(Debug, Clone, Copy)]
pub struct ImageParts<'image> {
    pub response: ImageResponseView<'image>,
    pub quality: Option<&'image str>,
    pub image_count: Option<i64>,
    pub size: Option<&'image str>,
}

pub struct RealtimeSession<'session> {
    pub results: &'session [Event],
    pub usage: Option<&'session Usage>,
}

// What completion_cost holds when it reaches the early-return stage, minus the usage B1688 already prepared.
pub struct SpecialRequest<'request> {
    pub call_type: CallType,
    pub passthrough_image: bool,
    pub prompt: &'request str,
    pub messages: &'request [Json],
    pub optional_params: Option<&'request IndexMap<String, Json>>,
    pub custom_pricing: Option<bool>,
    pub router_model_id: Option<&'request str>,
    pub base_model: Option<&'request str>,
    pub selected_model: Option<&'request str>,
    pub service_tier: Option<&'request str>,
    pub data_residency: Option<&'request str>,
    pub total_time: Option<Decimal>,
    pub moment: DateTime<Utc>,
    pub logging: LoggingView<'request>,
    pub response: SpecialResponse<'request>,
}

pub struct LoggingView<'logging> {
    pub model: Option<&'logging str>,
    pub params: Option<&'logging IndexMap<String, Json>>,
    pub timing: Option<ResponseTiming<'logging>>,
}

// The response attributes each branch reads; a branch whose response kind did not arrive receives the empty default.
pub struct SpecialResponse<'response> {
    pub cost_response: Option<&'response CostResponse<'response>>,
    pub image: ImageParts<'response>,
    pub video_usage: Option<VideoUsage<'response>>,
    pub transcription: TranscriptionDuration,
    pub rerank: Option<RerankUnits>,
    pub realtime: Option<RealtimeSession<'response>>,
    pub protocol: Option<&'response ProtocolCall>,
    pub replicate: ReplicateStamps,
    pub model_response: Option<&'response ResponseText<'response>>,
}

pub struct SpecialServices<'services, Resolver, ImagePolicy> {
    pub lookup: &'services dyn PriceLookup,
    pub resolver: &'services Resolver,
    pub image_policy: &'services ImagePolicy,
    pub special_policy: &'services dyn SpecialPricePolicy,
    pub size_pattern: &'services SizePattern,
    pub database: &'services TimeZoneDatabase,
    pub replicate_models: &'services ProviderModelSets,
    pub replicate_price: Decimal,
    pub discount: &'services BTreeMap<String, AdjustmentEntry>,
    pub margin: &'services BTreeMap<String, AdjustmentEntry>,
}

// The later token stage of B068 continues from here with the attempt this stage updated.
pub struct TokenStage {
    pub attempt: PreparedBillingAttempt,
    pub response_time_ms: Decimal,
    pub request_model_for_cost: Option<String>,
}

pub enum SpecialOutcome {
    Charged(Decimal),
    // A failed preparation is B1688's error to raise, so nothing here runs for it.
    Failed(PreparedBillingAttempt),
    Continue(Box<TokenStage>),
}
