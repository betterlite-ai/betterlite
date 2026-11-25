use std::cmp::Ordering;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_model_info::discovery::holds;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Number;

use super::datetime::normalize_model_info_datetime;

#[derive(Debug, thiserror::Error)]
pub enum SortError {
    #[error("model_info is not an object")]
    ModelInfoNotObject,
    #[error("model_name is not a string")]
    ModelNameNotText,
    #[error("model_info.team_public_model_name is not a string")]
    PublicNameNotText,
    #[error("a token price is not a number")]
    CostNotNumber,
    #[error("adding the token prices overflowed")]
    CostOverflow,
    #[error("model_info.db_model is not a boolean")]
    DbModelNotBool,
}

#[derive(Clone, Copy)]
enum SortField {
    Name,
    CreatedAt,
    UpdatedAt,
    Costs,
    Status,
}

// _sort_models: only the five named fields sort, an unknown one keeps the input order, and only desc (any case) reverses.
#[must_use]
pub fn sort_models(
    models: &[JsonObject],
    sort_by: Option<&str>,
    sort_order: Option<&str>,
) -> Vec<JsonObject> {
    let Some(label) = sort_by else {
        return models.to_vec();
    };
    let Some(field) = sort_field(label) else {
        return models.to_vec();
    };
    let reverse = sort_order.is_some_and(|order| order.eq_ignore_ascii_case("desc"));
    let mut keyed: Vec<(SortKey, JsonObject)> = Vec::with_capacity(models.len());
    for model in models {
        let key = match sort_key(model, field) {
            Ok(key) => key,
            Err(error) => {
                tracing::error!("Error sorting models by {label}: {error}");
                return models.to_vec();
            }
        };
        keyed.push((key, model.clone()));
    }
    keyed.sort_by(|(left, _), (right, _)| compare_sort_keys(left, right, reverse));
    keyed.into_iter().map(|(_, model)| model).collect()
}

fn sort_field(field: &str) -> Option<SortField> {
    match field {
        "model_name" => Some(SortField::Name),
        "created_at" => Some(SortField::CreatedAt),
        "updated_at" => Some(SortField::UpdatedAt),
        "costs" => Some(SortField::Costs),
        "status" => Some(SortField::Status),
        _ => None,
    }
}

// A date or cost the source cannot read sorts after every readable value in both orders, as LiteLLM's max and inf sentinels do.
#[derive(Debug, PartialEq, Eq)]
enum SortKey {
    Last,
    Name(String),
    Time(DateTime<Utc>),
    Cost(Decimal),
    Status(bool),
}

impl Ord for SortKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Name(left), Self::Name(right)) => left.cmp(right),
            (Self::Time(left), Self::Time(right)) => left.cmp(right),
            (Self::Cost(left), Self::Cost(right)) => left.cmp(right),
            (Self::Status(left), Self::Status(right)) => left.cmp(right),
            (Self::Last, Self::Last) => Ordering::Equal,
            (Self::Last, Self::Name(_) | Self::Time(_) | Self::Cost(_) | Self::Status(_)) => {
                Ordering::Greater
            }
            (Self::Name(_) | Self::Time(_) | Self::Cost(_) | Self::Status(_), Self::Last) => {
                Ordering::Less
            }
            (Self::Name(_), Self::Time(_) | Self::Cost(_) | Self::Status(_))
            | (Self::Time(_), Self::Name(_) | Self::Cost(_) | Self::Status(_))
            | (Self::Cost(_), Self::Name(_) | Self::Time(_) | Self::Status(_))
            | (Self::Status(_), Self::Name(_) | Self::Time(_) | Self::Cost(_)) => Ordering::Equal,
        }
    }
}

impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn compare_sort_keys(left: &SortKey, right: &SortKey, reverse: bool) -> Ordering {
    if matches!(left, SortKey::Last) || matches!(right, SortKey::Last) {
        return left.cmp(right);
    }
    let natural = left.cmp(right);
    if reverse { natural.reverse() } else { natural }
}

fn sort_key(model: &JsonObject, field: SortField) -> Result<SortKey, SortError> {
    let empty = JsonObject::new();
    let info = match model.get("model_info") {
        None | Some(Json::Null) => &empty,
        Some(Json::Object(fields)) => fields,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(SortError::ModelInfoNotObject);
        }
    };
    match field {
        SortField::Name => model_name_key(model, info),
        SortField::CreatedAt => Ok(time_key(info.get("created_at"))),
        SortField::UpdatedAt => Ok(time_key(info.get("updated_at"))),
        SortField::Costs => cost_key(info),
        SortField::Status => status_key(info),
    }
}

fn model_name_key(model: &JsonObject, info: &JsonObject) -> Result<SortKey, SortError> {
    let public = info.get("team_public_model_name");
    if public.is_some_and(holds) {
        let Some(Json::Text(name)) = public else {
            return Err(SortError::PublicNameNotText);
        };
        return Ok(SortKey::Name(name.to_lowercase()));
    }
    Ok(SortKey::Name(model_name(model)?.to_lowercase()))
}

fn model_name(model: &JsonObject) -> Result<&str, SortError> {
    match model.get("model_name") {
        None | Some(Json::Null) => Ok(""),
        Some(Json::Text(name)) => Ok(name),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(SortError::ModelNameNotText)
        }
    }
}

fn time_key(value: Option<&Json>) -> SortKey {
    value
        .and_then(normalize_model_info_datetime)
        .map_or(SortKey::Last, SortKey::Time)
}

fn cost_key(info: &JsonObject) -> Result<SortKey, SortError> {
    let input = decimal_cost(info.get("input_cost_per_token"))?;
    let output = decimal_cost(info.get("output_cost_per_token"))?;
    let total = input.checked_add(output).ok_or(SortError::CostOverflow)?;
    Ok(if total.is_zero() {
        SortKey::Last
    } else {
        SortKey::Cost(total)
    })
}

fn decimal_cost(value: Option<&Json>) -> Result<Decimal, SortError> {
    match value {
        None | Some(Json::Null) => Ok(Decimal::ZERO),
        Some(Json::Number(number)) => decimal_of(number),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(SortError::CostNotNumber)
        }
    }
}

fn decimal_of(number: &Number) -> Result<Decimal, SortError> {
    let text = number.to_string();
    text.parse::<Decimal>()
        .or_else(|_error| Decimal::from_scientific(&text))
        .map_err(|_error| SortError::CostNotNumber)
}

fn status_key(info: &JsonObject) -> Result<SortKey, SortError> {
    match info.get("db_model") {
        None | Some(Json::Null) => Ok(SortKey::Status(false)),
        Some(Json::Bool(flag)) => Ok(SortKey::Status(*flag)),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(SortError::DbModelNotBool)
        }
    }
}
