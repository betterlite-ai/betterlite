mod scripted;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::mem::take;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::cost::completion::lookup::{CatalogLookup, LookupSettings};
use betterlite_core::relay::assemblyai::poll::TokioSleeper;
use betterlite_core::relay::assemblyai::{
    AssemblyAiLogging, AssemblyAiPorts, AssemblyAiSettings, AssemblyLogCall,
    assembly_base_url_from_region, assembly_region_from_url,
};
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_metadata_resolution::names::NameNormalizers;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::{DateTime, Utc};
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio_util::task::TaskTracker;

use scripted::{Recorded, RecordingSink, Reply, ScriptedTransport, StaticCredentials};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    method: String,
    url_route: String,
    response_body: Json,
    transcripts: Vec<Reply>,
    catalog: IndexMap<String, Json>,
    api_key: Option<String>,
    polling_interval_seconds: u64,
    max_polling_attempts: u32,
}

struct Outcome {
    requests: Vec<Recorded>,
    model: Option<String>,
    provider: Option<&'static str>,
    cost: Option<rust_decimal::Decimal>,
}

const fn settings_of(call: &Call) -> AssemblyAiSettings {
    AssemblyAiSettings {
        polling_base_url: "https://api.assemblyai.com",
        polling_eu_base_url: "https://eu.assemblyai.com",
        polling_interval: Duration::from_secs(call.polling_interval_seconds),
        max_polling_attempts: call.max_polling_attempts,
    }
}

fn log_call(call: &Call) -> AssemblyLogCall {
    AssemblyLogCall {
        request_method: call.method.clone(),
        response_body: call.response_body.clone(),
        url_route: Some(call.url_route.clone()),
        result: "success".to_owned(),
        start_time: DateTime::<Utc>::UNIX_EPOCH.fixed_offset(),
        end_time: DateTime::<Utc>::UNIX_EPOCH.fixed_offset(),
        cache_hit: false,
        kwargs: PassThroughLoggingKwargs::default(),
    }
}

fn lookup_settings() -> LookupSettings {
    LookupSettings {
        known_model_names: BTreeSet::new(),
        custom_providers: BTreeSet::new(),
        environment: BTreeMap::new(),
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
        normalizers: NameNormalizers {
            bedrock_base_model: get_bedrock_base_model,
            bedrock_routing_prefix: strip_bedrock_routing_prefix,
            mantle_region_split: split_region_prefix,
        },
    }
}

fn outcome_of(transport: &ScriptedTransport, sink: &RecordingSink) -> Outcome {
    let requests = take(
        &mut *transport
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    let details = sink
        .logs
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .map(|log| {
            (
                Some(log.spend.model.clone()),
                Some(log.spend.custom_llm_provider),
                log.spend.response_cost,
            )
        });
    let (model, provider, cost) = details.unwrap_or((None, None, None));
    Outcome {
        requests,
        model,
        provider,
        cost,
    }
}

fn poll(call: &Call) -> Result<Outcome, Box<HelperError>> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)?;
    let transport = ScriptedTransport::new(&call.transcripts);
    let sink = RecordingSink::default();
    let bytes = serde_json::to_vec(&call.catalog).map_err(failure)?;
    let pricing = CatalogLookup::new(&bytes, lookup_settings()).map_err(failure)?;
    let service = Arc::new(AssemblyAiLogging::new(
        AssemblyAiPorts {
            credentials: StaticCredentials {
                key: call.api_key.as_deref().map(SecretString::from),
            },
            transport: transport.clone(),
            pricing,
            sleeper: TokioSleeper,
            sink: sink.clone(),
        },
        settings_of(call),
        TaskTracker::new(),
    ));
    runtime
        .block_on(service.run(&log_call(call)))
        .map_err(failure)?;
    Ok(outcome_of(&transport, &sink))
}

fn request_view(record: &Recorded) -> Value {
    let base = if record.url.host_str() == Some("eu.assemblyai.com") {
        "eu"
    } else {
        "us"
    };
    json!({
        "base": base,
        "path": record.url.path(),
        "authorization": record.headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok()),
        "content_type": record.headers.get(CONTENT_TYPE).and_then(|value| value.to_str().ok()),
    })
}

fn answer(call: &Call) -> Result<Value, Box<HelperError>> {
    let region = assembly_region_from_url(Some(call.url_route.as_str()));
    let routing_base = assembly_base_url_from_region(region);
    if call.method != "POST" {
        return Ok(json!({
            "logged": false,
            "attempts": 0,
            "requests": [],
            "model": null,
            "custom_llm_provider": null,
            "response_cost": null,
            "region": region,
            "routing_base": routing_base,
        }));
    }
    let outcome = poll(call)?;
    let views: Vec<Value> = outcome.requests.iter().map(request_view).collect();
    let cost = outcome.cost.map(|value| value.normalize().to_string());
    Ok(json!({
        "logged": true,
        "attempts": outcome.requests.len(),
        "requests": views,
        "model": outcome.model,
        "custom_llm_provider": outcome.provider,
        "response_cost": cost,
        "region": region,
        "routing_base": routing_base,
    }))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answered = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            answered.push(answer(call)?);
        }
        let mut result = serde_json::Map::new();
        result.insert("calls".to_owned(), Value::Array(answered));
        Ok(Value::Object(result))
    }
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
