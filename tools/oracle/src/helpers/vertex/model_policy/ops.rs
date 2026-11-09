use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::metadata::identity::AudioFormat;
use betterlite_models::metadata::vendor::VertexAudioApi;
use betterlite_vertex::model_policy::capabilities::{
    CapabilityLookupError, ModelCapabilityLookup, VertexCapabilityProvider,
    get_supports_response_schema, get_supports_system_message, supports_response_json_schema,
};
use betterlite_vertex::model_policy::lyria::{LyriaModelInfo, LyriaOverlay, lyria_model_key};
use betterlite_vertex::model_policy::partners::{
    is_vertex_partner_model, should_use_openai_handler,
};
use betterlite_vertex::model_policy::publisher::is_self_deployed_endpoint;
use betterlite_vertex::model_policy::route::{RouteParams, get_vertex_ai_model_route};
use betterlite_vertex::model_urls::route::VertexModelRoute;
use betterlite_vertex::provider_metadata::redaction::{
    redact_litellm_params_metadata, redact_logged_object_metadata,
};
use betterlite_vertex::provider_metadata::timestamp::{LocalTimeContext, decode_vertex_timestamp};
use betterlite_vertex::request_labels::{pop_vertex_request_labels, request_labels_from_params};
use jiff::tz::TimeZone;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::{Value, json};
use tracing::Subscriber;
use tracing::subscriber::with_default;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context as LayerContext;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::HelperError;

use super::input::{
    CapabilityCase, Input, LabelCase, LookupOutcome, LyriaCase, ProviderName, RouteCase,
};

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    match input {
        Input::Route { cases } => Ok(routes(cases)),
        Input::Partner { models } => Ok(partners(models)),
        Input::Capabilities { cases } => Ok(capabilities(cases)),
        Input::Lyria { cases } => Ok(lyria(cases)),
        Input::Labels { cases } => Ok(labels(cases)),
        Input::Metadata {
            logged_objects,
            call_details,
        } => Ok(metadata(logged_objects, call_details)),
        Input::Timestamp { zone, values } => timestamps(zone, values),
    }
}

fn routes(cases: &[RouteCase]) -> Value {
    let routes: Vec<&str> = cases
        .iter()
        .map(|case| {
            let params = RouteParams {
                base_model: case.base_model.as_deref(),
                api_base: case.api_base.as_deref(),
            };
            route_name(get_vertex_ai_model_route(&case.model, Some(&params)))
        })
        .collect();
    json!({"routes": routes})
}

const fn route_name(route: VertexModelRoute) -> &'static str {
    match route {
        VertexModelRoute::PartnerModels => "partner_models",
        VertexModelRoute::Gemini => "gemini",
        VertexModelRoute::Gemma => "gemma",
        VertexModelRoute::Bge => "bge",
        VertexModelRoute::ModelGarden => "model_garden",
        VertexModelRoute::NonGemini => "non_gemini",
        VertexModelRoute::OpenAi => "openai",
        VertexModelRoute::AgentEngine => "agent_engine",
    }
}

fn partners(models: &[String]) -> Value {
    let entries: Vec<Value> = models
        .iter()
        .map(|model| {
            json!({
                "model": model,
                "partner": is_vertex_partner_model(model),
                "openai_handler": should_use_openai_handler(model),
                "self_deployed": is_self_deployed_endpoint(model),
            })
        })
        .collect();
    json!({"models": entries})
}

fn capabilities(cases: &[CapabilityCase]) -> Value {
    let entries: Vec<Value> = cases.iter().map(capability_value).collect();
    json!({
        "cases": entries,
    })
}

fn capability_value(case: &CapabilityCase) -> Value {
    let lookup = FixtureLookup {
        system: case.system,
        schema: case.schema,
        provider: expected_provider(case.provider),
    };
    let counter = WarningCounter::default();
    let subscriber = tracing_subscriber::registry().with(counter.clone());
    let provider = into_provider(case.provider);
    let system = with_default(subscriber, || {
        get_supports_system_message(&case.model, provider, &lookup)
    });
    let schema = get_supports_response_schema(&case.model, provider, &lookup).map_or_else(
        |_error| json!({"error": true}),
        |supported| json!(supported),
    );
    json!({
        "system": system,
        "system_warned": counter.0.load(Ordering::Relaxed) > 0,
        "schema": schema,
        "json_schema": supports_response_json_schema(&case.model),
    })
}

const fn into_provider(name: ProviderName) -> VertexCapabilityProvider {
    match name {
        ProviderName::VertexAi => VertexCapabilityProvider::VertexAi,
        ProviderName::VertexAiBeta => VertexCapabilityProvider::VertexAiBeta,
        ProviderName::Gemini => VertexCapabilityProvider::Gemini,
    }
}

const fn expected_provider(name: ProviderName) -> &'static str {
    match name {
        ProviderName::VertexAi | ProviderName::VertexAiBeta => "vertex_ai",
        ProviderName::Gemini => "gemini",
    }
}

struct FixtureLookup {
    system: LookupOutcome,
    schema: LookupOutcome,
    provider: &'static str,
}

impl ModelCapabilityLookup for FixtureLookup {
    fn supports_system_messages(
        &self,
        _model: &str,
        provider: &str,
    ) -> Result<bool, CapabilityLookupError> {
        if provider != self.provider {
            return Err(CapabilityLookupError {
                message: format!("expected provider {}", self.provider),
            });
        }
        self.system.value()
    }

    fn supports_response_schema(
        &self,
        _model: &str,
        provider: &str,
    ) -> Result<bool, CapabilityLookupError> {
        if provider != self.provider {
            return Err(CapabilityLookupError {
                message: format!("expected provider {}", self.provider),
            });
        }
        self.schema.value()
    }
}

impl LookupOutcome {
    fn value(self) -> Result<bool, CapabilityLookupError> {
        match self {
            Self::True => Ok(true),
            Self::False => Ok(false),
            Self::Error => Err(CapabilityLookupError {
                message: "capability lookup failed".to_owned(),
            }),
        }
    }
}

#[derive(Clone, Default)]
struct WarningCounter(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for WarningCounter {
    fn on_event(&self, event: &tracing::Event<'_>, _context: LayerContext<'_, S>) {
        if event.metadata().level() == &tracing::Level::WARN {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn lyria(cases: &[LyriaCase]) -> Value {
    let catalog = BundledCatalog::default().get();
    let entries: Vec<Value> = cases
        .iter()
        .map(|case| lyria_value(case, catalog.as_ref()))
        .collect();
    json!({"cases": entries})
}

fn lyria_value(case: &LyriaCase, catalog: &ModelCatalog) -> Value {
    let mut runtime = BTreeMap::new();
    if let Some(entry) = &case.runtime {
        runtime.insert(lyria_model_key(&case.model), entry.clone());
    }
    let overlay = LyriaOverlay {
        runtime: &runtime,
        bundled: catalog,
    };
    let info = overlay.model_info(&case.model);
    let cost = overlay.generation_cost(&case.model);
    json!({
        "info": info.as_ref().map_or(Value::Null, info_value),
        "cost": cost.map_or(Value::Null, decimal_value),
    })
}

fn info_value(info: &LyriaModelInfo) -> Value {
    let formats: Vec<&str> = info
        .supported_audio_formats
        .iter()
        .map(|format| audio_format_name(*format))
        .collect();
    json!({
        "audio_api": audio_api_name(info.vertex_ai_audio_api),
        "formats": formats,
        "cost": info.output_cost_per_image.map_or(Value::Null, decimal_value),
    })
}

const fn audio_api_name(api: VertexAudioApi) -> &'static str {
    match api {
        VertexAudioApi::LyriaPredict => "lyria_predict",
        VertexAudioApi::LyriaInteractions => "lyria_interactions",
    }
}

const fn audio_format_name(format: AudioFormat) -> &'static str {
    match format {
        AudioFormat::Mp3 => "mp3",
        AudioFormat::Wav => "wav",
    }
}

fn decimal_value(amount: Decimal) -> Value {
    let Some(float) = amount.to_f64() else {
        return Value::Null;
    };
    json!(float)
}

fn labels(cases: &[LabelCase]) -> Value {
    let entries: Vec<Value> = cases.iter().map(label_value).collect();
    json!({
        "cases": entries,
    })
}

fn label_value(case: &LabelCase) -> Value {
    let mut optional_params = case.optional_params.clone();
    let labels = pop_vertex_request_labels(&mut optional_params, case.litellm_params.as_ref());
    let from_params = request_labels_from_params(case.litellm_params.as_ref());
    json!({
        "labels": labels,
        "from_params": from_params,
        "optional_params": optional_params,
    })
}

fn metadata(logged_objects: &[Json], call_details: &[Json]) -> Value {
    let mut logged = logged_objects.to_vec();
    for object in &mut logged {
        redact_logged_object_metadata(object);
    }
    let mut details = call_details.to_vec();
    for detail in &mut details {
        redact_litellm_params_metadata(detail);
    }
    json!({"logged_objects": logged, "call_details": details})
}

fn timestamps(zone: &str, values: &[String]) -> Result<Value, Box<HelperError>> {
    let time_zone = TimeZone::get(zone)
        .map_err(|error| invalid_input(format!("unknown time zone {zone}: {error}")))?;
    let context = LocalTimeContext::from(time_zone);
    let results: Vec<Value> = values
        .iter()
        .map(|value| {
            decode_vertex_timestamp(value.as_bytes(), &context).map_or_else(
                |_error| json!({"error": true}),
                |epoch| json!({"epoch": epoch}),
            )
        })
        .collect();
    Ok(json!({"timestamps": results}))
}

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
