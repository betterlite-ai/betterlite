use betterlite_db::health_check_rows::HealthCheckRow;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::health_access_and_connections::connection_error::RejectionDetail;
use betterlite_server::health_status_routes::convert::convert_health_check;
use betterlite_server::health_status_routes::error::HealthRouteError;
use betterlite_server::health_status_routes::target::resolve_target_model_name;
use betterlite_server::health_status_routes::testing::fake_health_router;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ConvertRow { rows } => convert_rows(rows),
            Input::ResolveTarget { cases } => resolve_targets(cases),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ConvertRow { rows: Vec<RowInput> },
    ResolveTarget { cases: Vec<CaseInput> },
}

#[derive(Debug, Deserialize)]
pub(crate) struct RowInput {
    health_check_id: String,
    model_name: String,
    #[serde(default)]
    model_id: Option<String>,
    status: String,
    healthy_count: i32,
    unhealthy_count: i32,
    #[serde(default)]
    error_message: Option<String>,
    #[serde(default)]
    response_time_ms: Option<f64>,
    #[serde(default)]
    details: Option<Json>,
    #[serde(default)]
    checked_by: Option<String>,
    checked_at: String,
    created_at: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CaseInput {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    router: Option<RouterInput>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RouterInput {
    #[serde(default)]
    deployments: Vec<DeploymentInput>,
    #[serde(default)]
    fail: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct DeploymentInput {
    model_name: String,
    model_id: String,
}

fn convert_rows(rows: &[RowInput]) -> HelperResult {
    let mut converted = Vec::new();
    for row in rows {
        converted.push(serde_json::to_value(convert_health_check(&health_row(
            row,
        )?))?);
    }
    Ok(Value::Array(converted))
}

fn health_row(input: &RowInput) -> Result<HealthCheckRow, Box<HelperError>> {
    let checked_at = instant(&input.checked_at)?;
    let created_at = instant(&input.created_at)?;
    Ok(HealthCheckRow {
        health_check_id: input.health_check_id.clone(),
        model_name: input.model_name.clone(),
        model_id: input.model_id.clone(),
        status: input.status.clone(),
        healthy_count: input.healthy_count,
        unhealthy_count: input.unhealthy_count,
        error_message: input.error_message.clone(),
        response_time_ms: input.response_time_ms,
        details: input.details.clone(),
        checked_by: input.checked_by.clone(),
        checked_at,
        created_at,
        updated_at: created_at,
    })
}

fn instant(text: &str) -> Result<DateTime<Utc>, Box<HelperError>> {
    DateTime::parse_from_rfc3339(text)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| invalid(error.to_string()))
}

fn resolve_targets(cases: &[CaseInput]) -> HelperResult {
    let mut answers = Vec::new();
    for case in cases {
        answers.push(target_answer(case)?);
    }
    Ok(Value::Array(answers))
}

fn target_answer(case: &CaseInput) -> HelperResult {
    let router = case.router.as_ref().map(|input| {
        fake_health_router(
            input
                .deployments
                .iter()
                .map(|deployment| (deployment.model_name.clone(), deployment.model_id.clone())),
            input.fail,
        )
    });
    match resolve_target_model_name(
        case.model.as_deref(),
        case.model_id.as_deref(),
        router.as_deref(),
    ) {
        Ok(model_name) => Ok(json!({ "model_name": model_name })),
        Err(HealthRouteError::Http(rejection)) => Ok(json!({
            "error": {
                "status": rejection.status.as_u16(),
                "detail": match &rejection.detail {
                    RejectionDetail::Message(text) => Value::String(text.clone()),
                    RejectionDetail::Error(text) => json!({ "error": text }),
                },
            },
        })),
        Err(error) => Err(invalid(error.to_string())),
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
