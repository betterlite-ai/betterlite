use betterlite_models::baseline_history::model::BaselineObservation;
use betterlite_models::savings_evidence::snapshot::BaselineCostSnapshot;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::autorouter_turn_sql::transaction::AutoRouterTurnTransaction;
use crate::daily_spend_sql::entities::{DailySpendEntity, SpendRow};
use crate::daily_spend_sql::values::SpendColumnValue;

pub mod canonical;
pub mod decode;
mod types;

// The daily entity a baseline correction lands on: its payload name and the column its rollup row is keyed by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaselineEntity {
    User,
    Team,
    Org,
    Tag,
    EndUser,
    Agent,
}

impl BaselineEntity {
    const fn names(self) -> EntityNames {
        match self {
            Self::User => EntityNames {
                name: "user",
                column: "user_id",
            },
            Self::Team => EntityNames {
                name: "team",
                column: "team_id",
            },
            Self::Org => EntityNames {
                name: "org",
                column: "organization_id",
            },
            Self::Tag => EntityNames {
                name: "tag",
                column: "tag",
            },
            Self::EndUser => EntityNames {
                name: "end_user",
                column: "end_user_id",
            },
            Self::Agent => EntityNames {
                name: "agent",
                column: "agent_id",
            },
        }
    }
}

// The payload name and the daily rollup column one entity is written under.
struct EntityNames {
    name: &'static str,
    column: &'static str,
}

impl From<BaselineEntity> for DailySpendEntity {
    fn from(value: BaselineEntity) -> Self {
        match value {
            BaselineEntity::User => Self::User,
            BaselineEntity::Team => Self::Team,
            BaselineEntity::Org => Self::Org,
            BaselineEntity::Tag => Self::Tag,
            BaselineEntity::EndUser => Self::EndUser,
            BaselineEntity::Agent => Self::Agent,
        }
    }
}

// The entity a daily baseline correction lands on and the row id that entity is keyed by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DailyBaselineTarget {
    pub entity: BaselineEntity,
    pub entity_id: Option<String>,
}

// The daily rollup identity a baseline request was logged under, before the per-entity rows are built.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct DailyBaselineAttribution {
    pub date: String,
    pub api_key: String,
    pub model: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub model_group: Option<String>,
    pub endpoint: Option<String>,
    pub mcp_namespaced_tool_name: Option<String>,
    pub targets: Vec<DailyBaselineTarget>,
}

impl DailyBaselineAttribution {
    // Only the savings column moves: the request, token and spend counters are never incremented here.
    #[must_use]
    pub fn adjustment(
        &self,
        target: &DailyBaselineTarget,
        savings_delta: Decimal,
        request_id: &str,
    ) -> SpendRow {
        let mut row = SpendRow::new();
        row.insert("date".to_owned(), SpendColumnValue::Text(self.date.clone()));
        row.insert(
            "api_key".to_owned(),
            SpendColumnValue::Text(self.api_key.clone()),
        );
        row.insert("model".to_owned(), optional_text(self.model.as_ref()));
        row.insert(
            "custom_llm_provider".to_owned(),
            optional_text(self.custom_llm_provider.as_ref()),
        );
        row.insert(
            "model_group".to_owned(),
            optional_text(self.model_group.as_ref()),
        );
        row.insert("endpoint".to_owned(), optional_text(self.endpoint.as_ref()));
        row.insert(
            "mcp_namespaced_tool_name".to_owned(),
            optional_text(self.mcp_namespaced_tool_name.as_ref()),
        );
        row.insert(
            target.entity.names().column.to_owned(),
            optional_text(target.entity_id.as_ref()),
        );
        row.insert(
            "request_id".to_owned(),
            SpendColumnValue::Text(request_id.to_owned()),
        );
        row.insert(
            "autorouter_savings_spend".to_owned(),
            SpendColumnValue::Decimal(savings_delta),
        );
        row
    }
}

fn optional_text(value: Option<&String>) -> SpendColumnValue {
    value.map_or(SpendColumnValue::Null, |text| {
        SpendColumnValue::Text(text.clone())
    })
}

// One stored autorouter baseline observation before publication; turn and daily are required nullable fields.
#[derive(Clone)]
pub struct BaselineAccountingRecord {
    pub scope: String,
    pub api_key: String,
    pub session_id: String,
    pub router_name: String,
    pub baseline_model: String,
    pub observation: BaselineObservation,
    pub pricing: BaselineCostSnapshot,
    pub turn: Option<AutoRouterTurnTransaction>,
    pub daily: Option<DailyBaselineAttribution>,
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("scope must be autorouter-baseline:v3: followed by 64 lowercase hex digits")]
    InvalidScope,
    #[error("api_key must not be empty")]
    EmptyApiKey,
    #[error("session_id must be 1 to 256 characters")]
    InvalidSessionId,
    #[error("router_name must not be empty")]
    EmptyRouterName,
    #[error("baseline_model must not be empty")]
    EmptyBaselineModel,
    #[error(
        "Baseline observation must own an unestimated turn with matching scope and actual cost"
    )]
    InconsistentTurn,
}

fn valid_scope(scope: &str) -> bool {
    let Some(digest) = scope.strip_prefix("autorouter-baseline:v3:") else {
        return false;
    };
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn consistent_turn(record: &BaselineAccountingRecord, turn: &AutoRouterTurnTransaction) -> bool {
    let identity_matches = turn.api_key == record.api_key
        && turn.session_id == record.session_id
        && turn.router_name == record.router_name
        && turn.baseline_model.as_deref() == Some(record.baseline_model.as_str());
    let cost_matches = record
        .pricing
        .actual_spend
        .checked_add(record.pricing.classifier_cost)
        .is_some_and(|expected| turn.spend == expected);
    identity_matches
        && cost_matches
        && turn.saved_spend.is_zero()
        && turn.savings_estimated_turns == 0
        && turn.savings_estimated_actual_spend.is_zero()
        && turn.savings_estimated_saved_spend.is_zero()
}

#[derive(Debug, thiserror::Error)]
pub enum RecordDecodeError {
    #[error("baseline record is not valid JSON of the required shape")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Invalid(RecordError),
}

// Stored data is the canonical text B1699 writes, so a retry decodes to the same record.
pub fn decode_baseline_record(data: &[u8]) -> Result<BaselineAccountingRecord, RecordDecodeError> {
    let raw: types::RawRecord = serde_json::from_slice(data)?;
    if !valid_scope(&raw.scope) {
        return Err(RecordDecodeError::Invalid(RecordError::InvalidScope));
    }
    if raw.api_key.is_empty() {
        return Err(RecordDecodeError::Invalid(RecordError::EmptyApiKey));
    }
    if !(1..=256).contains(&raw.session_id.chars().count()) {
        return Err(RecordDecodeError::Invalid(RecordError::InvalidSessionId));
    }
    if raw.router_name.is_empty() {
        return Err(RecordDecodeError::Invalid(RecordError::EmptyRouterName));
    }
    if raw.baseline_model.is_empty() {
        return Err(RecordDecodeError::Invalid(RecordError::EmptyBaselineModel));
    }
    let record = BaselineAccountingRecord {
        scope: raw.scope,
        api_key: raw.api_key,
        session_id: raw.session_id,
        router_name: raw.router_name,
        baseline_model: raw.baseline_model,
        observation: raw.observation,
        pricing: raw.pricing,
        turn: raw.turn,
        daily: raw.daily.map(types::RawDailyAttribution::into_attribution),
    };
    if let Some(turn) = &record.turn {
        if !consistent_turn(&record, turn) {
            return Err(RecordDecodeError::Invalid(RecordError::InconsistentTurn));
        }
    }
    Ok(record)
}

mod tests;
