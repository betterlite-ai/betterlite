use betterlite_db::health_check_rows::HealthCheckRow;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_server::health_persistence::writes::latest_by_key;
use betterlite_server::health_persistence::{
    AggregatedHealthResult, aggregate_results, model_param_index, should_persist,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ModelParamIndex { deployments } => index(deployments),
            Input::AggregateResults {
                deployments,
                healthy,
                unhealthy,
            } => aggregate(deployments, healthy, unhealthy),
            Input::PersistDecision {
                result,
                latest,
                now,
            } => decision(result, latest, now),
            Input::ResponseTime { values } => response_times(values),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Input {
    ModelParamIndex {
        deployments: Vec<DeploymentInput>,
    },
    AggregateResults {
        deployments: Vec<DeploymentInput>,
        #[serde(default)]
        healthy: Vec<JsonObject>,
        #[serde(default)]
        unhealthy: Vec<JsonObject>,
    },
    PersistDecision {
        result: ResultInput,
        #[serde(default)]
        latest: Vec<RowInput>,
        now: String,
    },
    ResponseTime {
        values: Vec<TimeInput>,
    },
}

#[derive(Debug, Deserialize)]
struct DeploymentInput {
    #[serde(default)]
    model_name: Option<String>,
    #[serde(default)]
    litellm_params: JsonObject,
    #[serde(default)]
    model_info: JsonObject,
}

#[derive(Debug, Deserialize)]
struct ResultInput {
    model_name: String,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    healthy_count: i32,
    #[serde(default)]
    unhealthy_count: i32,
    #[serde(default)]
    error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RowInput {
    #[serde(default)]
    health_check_id: String,
    model_name: String,
    #[serde(default)]
    model_id: Option<String>,
    status: String,
    checked_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TimeInput {
    Null,
    Number(f64),
    Text(String),
}

fn index(deployments: &[DeploymentInput]) -> HelperResult {
    let deployed = deployments.iter().map(spec).collect::<Vec<_>>();
    let mut output = serde_json::Map::new();
    for (param, models) in model_param_index(&deployed) {
        let listed = models
            .iter()
            .map(|model| {
                json!({
                    "model_name": model.model_name,
                    "model_id": model.model_id,
                })
            })
            .collect::<Vec<_>>();
        output.insert(param, Value::Array(listed));
    }
    Ok(Value::Object(output))
}

fn aggregate(
    deployments: &[DeploymentInput],
    healthy: &[JsonObject],
    unhealthy: &[JsonObject],
) -> HelperResult {
    let deployed = deployments.iter().map(spec).collect::<Vec<_>>();
    let results = aggregate_results(&model_param_index(&deployed), healthy, unhealthy);
    let entries = results
        .iter()
        .map(|result| {
            json!({
                "model_name": result.model_name,
                "model_id": result.model_id,
                "healthy_count": result.healthy_count,
                "unhealthy_count": result.unhealthy_count,
                "error_message": result.error_message,
            })
        })
        .collect::<Vec<_>>();
    Ok(Value::Array(entries))
}

fn decision(result: &ResultInput, latest: &[RowInput], now: &str) -> HelperResult {
    let aggregated = AggregatedHealthResult {
        model_name: result.model_name.clone(),
        model_id: result.model_id.clone(),
        healthy_count: result.healthy_count,
        unhealthy_count: result.unhealthy_count,
        error_message: result.error_message.clone(),
    };
    let rows = latest.iter().map(row).collect::<Result<Vec<_>, _>>()?;
    let instant = DateTime::parse_from_rfc3339(now)
        .map_err(|error| invalid(error.to_string()))?
        .with_timezone(&Utc);
    Ok(Value::Bool(should_persist(
        &aggregated,
        &latest_by_key(rows),
        instant,
    )))
}

fn row(input: &RowInput) -> Result<HealthCheckRow, Box<HelperError>> {
    let checked_at = DateTime::parse_from_rfc3339(&input.checked_at)
        .map_err(|error| invalid(error.to_string()))?
        .with_timezone(&Utc);
    Ok(HealthCheckRow {
        health_check_id: input.health_check_id.clone(),
        model_name: input.model_name.clone(),
        model_id: input.model_id.clone(),
        status: input.status.clone(),
        healthy_count: 0,
        unhealthy_count: 0,
        error_message: None,
        response_time_ms: None,
        details: None,
        checked_by: None,
        checked_at,
        created_at: checked_at,
        updated_at: checked_at,
    })
}

fn response_times(values: &[TimeInput]) -> HelperResult {
    let mut results = Vec::new();
    for value in values {
        // PrismaClient._validate_response_time drops a non-finite time without a warning
        let response_time = time(value)?.filter(|millis| millis.is_finite());
        results.push(json!(response_time));
    }
    Ok(Value::Array(results))
}

fn time(input: &TimeInput) -> Result<Option<f64>, Box<HelperError>> {
    match input {
        TimeInput::Null => Ok(None),
        TimeInput::Number(number) => Ok(Some(*number)),
        TimeInput::Text(text) => special(text).map(Some),
    }
}

fn special(text: &str) -> Result<f64, Box<HelperError>> {
    match text {
        "nan" => Ok(f64::NAN),
        "inf" => Ok(f64::INFINITY),
        "-inf" => Ok(f64::NEG_INFINITY),
        _ => Err(invalid(format!("unknown response time: {text}"))),
    }
}

fn spec(input: &DeploymentInput) -> DeploymentSpec {
    DeploymentSpec {
        model_name: input.model_name.clone().unwrap_or_default(),
        litellm_params: input.litellm_params.clone(),
        model_info: input.model_info.clone(),
        extra: JsonObject::new(),
    }
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
