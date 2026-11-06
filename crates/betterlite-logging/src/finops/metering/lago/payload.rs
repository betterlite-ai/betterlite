use rust_decimal::Decimal;
use serde::Serialize;
use uuid::Uuid;

use crate::finops::metering::event::MeteringEvent;
use crate::finops::metering::lago::settings::{ChargeBy, LagoSettings};

#[derive(Debug, Serialize)]
pub struct LagoPayload {
    pub event: LagoEvent,
}

#[derive(Debug, Serialize)]
pub struct LagoEvent {
    pub transaction_id: String,
    pub external_subscription_id: String,
    pub code: String,
    pub properties: LagoProperties,
}

#[derive(Debug, Serialize)]
pub struct LagoProperties {
    pub model: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub response_cost: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum LagoPayloadError {
    #[error(
        "external customer ID is not set for charge_by={charge_by:?}: user_id={user_id:?} end_user_id={end_user_id:?} team_id={team_id:?}"
    )]
    MissingSubscription {
        charge_by: ChargeBy,
        user_id: Option<String>,
        end_user_id: Option<String>,
        team_id: Option<String>,
    },
}

pub fn build(
    settings: &LagoSettings,
    event: &MeteringEvent<'_>,
) -> Result<LagoPayload, LagoPayloadError> {
    let external_subscription_id = match settings.charge_by {
        ChargeBy::EndUserId => event.request_user,
        ChargeBy::TeamId => event.user_api_key_team_id,
        ChargeBy::UserId => event.user_api_key_user_id,
    };
    let Some(subscription_id) = external_subscription_id else {
        return Err(LagoPayloadError::MissingSubscription {
            charge_by: settings.charge_by,
            user_id: event.user_api_key_user_id.map(str::to_owned),
            end_user_id: event.request_user.map(str::to_owned),
            team_id: event.user_api_key_team_id.map(str::to_owned),
        });
    };
    Ok(LagoPayload {
        event: LagoEvent {
            transaction_id: Uuid::new_v4().to_string(),
            external_subscription_id: subscription_id.to_owned(),
            code: settings.event_code.clone(),
            properties: LagoProperties {
                model: event.model.map(str::to_owned),
                response_cost: event.response_cost,
                prompt_tokens: event.usage.map(|usage| usage.prompt_tokens),
                completion_tokens: event.usage.map(|usage| usage.completion_tokens),
                total_tokens: event.usage.map(|usage| usage.total_tokens),
            },
        },
    })
}
