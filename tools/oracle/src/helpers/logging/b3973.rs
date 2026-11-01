use std::collections::BTreeMap;

use betterlite_logging::opik::config::get_config_variable;
use betterlite_logging::opik::config_file::ConfigFileError;
use betterlite_logging::opik::metadata::extract_opik_metadata;
use betterlite_logging::opik::payload::{
    PayloadContent, PayloadWindow, SpanContext, SpanDetails, build_span_payload,
};
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct ConfigLookup {
    key: String,
    user: Option<String>,
    default: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct PayloadInput {
    project_name: String,
    trace_id: String,
    parent_span_id: Option<String>,
    response: IndexMap<String, Json>,
    input: Json,
    output: Json,
    #[serde(default)]
    metadata: IndexMap<String, Json>,
    #[serde(default)]
    tags: Vec<Json>,
    provider: Option<String>,
    cost: Option<Json>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Uuid,
    Config {
        #[serde(default)]
        environment: BTreeMap<String, String>,
        config_file: Option<String>,
        lookups: Vec<ConfigLookup>,
    },
    Metadata {
        #[serde(default)]
        litellm_metadata: IndexMap<String, Json>,
        #[serde(default)]
        standard_logging_metadata: IndexMap<String, Json>,
    },
    Payload(Box<PayloadInput>),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Uuid => run_uuid(),
            Input::Config {
                environment,
                config_file,
                lookups,
            } => run_config(environment, config_file.as_deref(), lookups),
            Input::Metadata {
                litellm_metadata,
                standard_logging_metadata,
            } => Ok(json!({
                "metadata": extract_opik_metadata(litellm_metadata, standard_logging_metadata),
            })),
            Input::Payload(payload) => Ok(run_payload(payload)),
        }
    }
}

fn run_uuid() -> HelperResult {
    let created = betterlite_logging::opik::uuid7::create_uuid7();
    let Ok(parsed) = uuid::Uuid::parse_str(&created) else {
        return Err(Box::new(HelperError {
            class: "Uuid".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "create_uuid7 returned a malformed UUID".to_owned(),
        }));
    };
    let variant = parsed.as_bytes().get(8).copied().unwrap_or_default() >> 6_u8;
    let canonical = parsed.to_string() == created;
    Ok(json!({
        "version": parsed.get_version_num(),
        "variant": variant,
        "canonical": canonical,
    }))
}

fn run_config(
    environment: &BTreeMap<String, String>,
    config_file: Option<&str>,
    lookups: &[ConfigLookup],
) -> HelperResult {
    let mut values = Vec::with_capacity(lookups.len());
    for lookup in lookups {
        let value = get_config_variable(
            &lookup.key,
            lookup.user.as_deref(),
            environment,
            config_file,
            lookup.default.as_deref(),
        )
        .map_err(|error: ConfigFileError| {
            Box::new(HelperError {
                class: "ConfigFileError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        values.push(value);
    }
    Ok(json!({"values": values}))
}

fn run_payload(payload: &PayloadInput) -> serde_json::Value {
    let start =
        DateTime::<Utc>::from_timestamp(1_767_225_600, 0).unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    let end =
        DateTime::<Utc>::from_timestamp(1_767_225_601, 0).unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    let scope = SpanContext {
        project_name: &payload.project_name,
        trace_id: &payload.trace_id,
        parent_span_id: payload.parent_span_id.clone(),
        response: &payload.response,
    };
    let window = PayloadWindow { start, end };
    let body = PayloadContent {
        input: payload.input.clone(),
        output: payload.output.clone(),
        metadata: payload.metadata.clone(),
        tags: payload.tags.clone(),
    };
    let details = SpanDetails {
        cost: payload.cost.clone(),
        provider: payload.provider.as_deref(),
    };
    let mut span = build_span_payload(scope, window, body, details);
    for key in ["id", "start_time", "end_time"] {
        span.shift_remove(key);
    }
    json!(span)
}
