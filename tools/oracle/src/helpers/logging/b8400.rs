use std::collections::BTreeMap;
use std::io::Error as IoError;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use betterlite_server::proxy_helpers::checks::{
    Detail, ErrorText, HttpException, ProxyCheckError, check_team_models, error_message_str,
    premium_user_check, prisma_client_or_throw,
};
use betterlite_server::proxy_helpers::projected::{
    ProjectedLimit, ProjectionError, is_projected_spend_over_limit, month_end_date,
    projected_spend_over_limit,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
enum Call {
    ErrorMessage {
        status: u16,
        #[serde(default)]
        detail: Option<DetailWire>,
        #[serde(default)]
        message: Option<DetailWire>,
    },
    PlainErrorMessage {
        text: String,
    },
    TeamModels {
        #[serde(default)]
        team_id: Option<String>,
        #[serde(default)]
        team_config: Option<BTreeMap<String, Json>>,
        #[serde(default)]
        request_data: Option<BTreeMap<String, Json>>,
    },
    MonthEnd {
        today: NaiveDate,
    },
    Projection {
        spend: String,
        #[serde(default)]
        soft_limit: Option<String>,
        today: NaiveDate,
    },
    Premium {
        premium_user: bool,
        #[serde(default)]
        feature: Option<String>,
    },
    Database {
        connected: bool,
        message: String,
    },
}

// an absent detail is the case's way of saying LiteLLM reads the message attribute instead
#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum DetailWire {
    Text(String),
    Map(BTreeMap<String, Json>),
}

impl DetailWire {
    fn into_detail(self) -> Detail {
        match self {
            Self::Text(text) => Detail::Text(text),
            Self::Map(map) => Detail::Map(map),
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            results.push(perform(call)?);
        }
        Ok(json!({ "results": results }))
    }
}

fn perform(call: &Call) -> HelperResult {
    match call {
        Call::ErrorMessage {
            status,
            detail,
            message,
        } => error_message(*status, detail.as_ref(), message.as_ref()),
        Call::PlainErrorMessage { text } => Ok(json!(error_message_str(ErrorText::Other(
            &IoError::other(text.clone())
        )))),
        Call::TeamModels {
            team_id,
            team_config,
            request_data,
        } => Ok(json!(team_models(
            team_id.as_deref(),
            team_config.as_ref(),
            request_data.as_ref()
        ))),
        Call::MonthEnd { today } => {
            Ok(json!(month_end_date(*today).format("%Y-%m-%d").to_string()))
        }
        Call::Projection {
            spend,
            soft_limit,
            today,
        } => projection(spend, soft_limit.as_deref(), *today),
        Call::Premium {
            premium_user,
            feature,
        } => Ok(json!(premium(*premium_user, feature.as_deref()))),
        Call::Database { connected, message } => Ok(json!(database(*connected, message))),
    }
}

fn error_message(
    status: u16,
    detail: Option<&DetailWire>,
    message: Option<&DetailWire>,
) -> HelperResult {
    let code = StatusCode::from_u16(status).map_err(|error| failure(&error.to_string()))?;
    let http = HttpException {
        status: code,
        detail: detail.cloned().map(DetailWire::into_detail),
        message: message.cloned().map(DetailWire::into_detail),
    };
    Ok(json!(error_message_str(ErrorText::Http(&http))))
}

fn team_models(
    team_id: Option<&str>,
    team_config: Option<&BTreeMap<String, Json>>,
    request_data: Option<&BTreeMap<String, Json>>,
) -> Value {
    let mut config = team_config.cloned();
    let outcome = check_team_models(team_id, config.as_mut(), request_data);
    json!({ "result": outcome_name(&outcome), "team_config": config })
}

fn projection(spend: &str, soft_limit: Option<&str>, today: NaiveDate) -> HelperResult {
    let spend_amount = parse_amount(spend)?;
    let limit_value = soft_limit.map(parse_amount).transpose()?;
    let over = is_projected_spend_over_limit(spend_amount, limit_value, today)
        .map_err(|error: ProjectionError| failure(&error.to_string()))?;
    let reached = projected_spend_over_limit(spend_amount, limit_value, today)
        .map_err(|error: ProjectionError| failure(&error.to_string()))?;
    let limit = reached
        .as_ref()
        .map(limit_json)
        .transpose()?
        .unwrap_or(Value::Null);
    Ok(json!({ "over_limit": over, "limit": limit }))
}

fn premium(premium_user: bool, feature: Option<&str>) -> Value {
    match premium_user_check(premium_user, feature) {
        Ok(()) => json!({ "allowed": true }),
        Err(error) => json!({
            "allowed": false,
            "status": error.status_code().as_u16(),
            "detail": error.detail(),
        }),
    }
}

fn database(connected: bool, message: &str) -> Value {
    let client = connected.then_some(());
    match prisma_client_or_throw(client.as_ref(), message) {
        Ok(_client) => json!({ "connected": true }),
        Err(error) => json!({
            "connected": false,
            "status": error.status_code().as_u16(),
            "detail": error.detail(),
        }),
    }
}

const fn outcome_name(outcome: &Result<(), ProxyCheckError>) -> &'static str {
    match outcome {
        Ok(()) => "ok",
        Err(ProxyCheckError::InvalidTeamModel { .. }) => "invalid_model",
        Err(ProxyCheckError::MissingRequestModel) => "missing_model",
        Err(ProxyCheckError::InvalidTeamModels) => "invalid_models",
        Err(ProxyCheckError::PremiumUser { .. }) => "premium_user",
        Err(ProxyCheckError::Database { .. }) => "database",
    }
}

fn parse_amount(text: &str) -> Result<Decimal, Box<HelperError>> {
    text.parse()
        .map_err(|error: rust_decimal::Error| failure(&error.to_string()))
}

fn limit_json(limit: &ProjectedLimit) -> Result<Value, Box<HelperError>> {
    Ok(json!({
        "projected_spend": float_text(limit.projected_spend)?,
        "date": limit.limit_exceed_date.format("%Y-%m-%d").to_string(),
    }))
}

fn float_text(value: Decimal) -> Result<String, Box<HelperError>> {
    let number = value
        .to_f64()
        .ok_or_else(|| failure("the projected spend does not fit an f64"))?;
    let mut text = format!("{number}");
    if !text.contains('.') && !text.contains('e') {
        text.push_str(".0");
    }
    Ok(text)
}

fn failure(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "HelperFailure".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
