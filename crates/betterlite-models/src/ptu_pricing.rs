use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use rust_decimal::Decimal;

use crate::savings_evidence::billed_at::decode_billed_at;
use crate::tiered_pricing::Scalar;

pub mod pricing_fields;
pub mod spillover;
pub mod zeroed;

mod numeric;
mod tests;

pub const PTU_COST_ATTRIBUTION_ENV_VAR: &str = "LITELLM_ENABLE_PTU_COST_ATTRIBUTION";
pub const MAX_PTU_COUNT: u32 = 1_000_000;
pub const MAX_COST_PER_PTU_PER_HOUR: Decimal = Decimal::from_parts(1_000_000, 0, 0, false, 0);

const INVERTED_WINDOW: &str = "ptu_effective_to must be after ptu_effective_from";
const UNPAIRED_COUNT_RATE: &str = "ptu_count and cost_per_ptu_per_hour must be set together";
const MISSING_START: &str = "ptu_effective_from is required when PTU fields are set. Flat cost \
     accrues from that instant, so without it the start would have to be inferred and a deployment \
     configured today could be billed for days it did not exist";
const MISSING_TEAM: &str =
    "team_id is required when PTU fields are set (one model maps to one team)";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PtuTerms {
    pub team_id: String,
    pub ptu_count: u32,
    pub cost_per_ptu_per_hour: Decimal,
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
}

// A model_info bound as LiteLLM's _as_utc reads it: a string, a typed YAML date, or a naive or aware datetime.
#[derive(Clone, Copy, Debug)]
pub enum PtuInstantSource<'value> {
    Scalar(&'value Scalar),
    Date(NaiveDate),
    NaiveDateTime(NaiveDateTime),
    DateTime(DateTime<FixedOffset>),
}

impl PtuInstantSource<'_> {
    fn as_utc(self) -> Option<DateTime<Utc>> {
        match self {
            Self::Scalar(Scalar::Text(text)) => decode_billed_at(text),
            Self::Scalar(_) => None,
            Self::Date(date) => Some(date.and_time(NaiveTime::MIN).and_utc()),
            Self::NaiveDateTime(naive) => Some(naive.and_utc()),
            Self::DateTime(aware) => Some(aware.with_timezone(&Utc)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PtuModelInfo<'value> {
    pub ptu_count: Option<&'value Scalar>,
    pub cost_per_ptu_per_hour: Option<&'value Scalar>,
    pub team_id: Option<&'value Scalar>,
    pub ptu_effective_from: Option<PtuInstantSource<'value>>,
    pub ptu_effective_to: Option<PtuInstantSource<'value>>,
}

impl<'value> From<&'value BTreeMap<String, Scalar>> for PtuModelInfo<'value> {
    fn from(model_info: &'value BTreeMap<String, Scalar>) -> Self {
        Self {
            ptu_count: model_info.get("ptu_count"),
            cost_per_ptu_per_hour: model_info.get("cost_per_ptu_per_hour"),
            team_id: model_info.get("team_id"),
            ptu_effective_from: model_info
                .get("ptu_effective_from")
                .map(PtuInstantSource::Scalar),
            ptu_effective_to: model_info
                .get("ptu_effective_to")
                .map(PtuInstantSource::Scalar),
        }
    }
}

#[must_use]
pub fn ptu_terms(model_info: &PtuModelInfo<'_>) -> Option<PtuTerms> {
    let team_id = team_id_text(model_info.team_id)?;
    let ptu_count = numeric::ptu_count(model_info.ptu_count?)?;
    let cost_per_ptu_per_hour = numeric::ptu_rate(model_info.cost_per_ptu_per_hour?)?;
    if cost_per_ptu_per_hour < Decimal::ZERO || cost_per_ptu_per_hour > MAX_COST_PER_PTU_PER_HOUR {
        return None;
    }

    let raw_to = model_info
        .ptu_effective_to
        .filter(|source| !matches!(source, PtuInstantSource::Scalar(Scalar::Null)));
    let effective_from = model_info.ptu_effective_from?.as_utc()?;
    let effective_to = match raw_to {
        Some(source) => Some(source.as_utc()?),
        None => None,
    };
    if effective_to.is_some_and(|end| end <= effective_from) {
        return None;
    }
    Some(PtuTerms {
        team_id: team_id.to_owned(),
        ptu_count,
        cost_per_ptu_per_hour,
        effective_from,
        effective_to,
    })
}

#[must_use]
pub fn ptu_config_error(model_info: &PtuModelInfo<'_>, model_name: Option<&str>) -> Option<String> {
    if inverted_window(model_info) {
        return Some(named(INVERTED_WINDOW, model_name));
    }
    declaration_gaps(model_info).map(|reason| named(reason, model_name))
}

fn inverted_window(model_info: &PtuModelInfo<'_>) -> bool {
    let start = model_info
        .ptu_effective_from
        .and_then(PtuInstantSource::as_utc);
    let end = model_info
        .ptu_effective_to
        .and_then(PtuInstantSource::as_utc);
    matches!((start, end), (Some(from), Some(to)) if to <= from)
}

fn declaration_gaps(model_info: &PtuModelInfo<'_>) -> Option<&'static str> {
    let has_count = model_info
        .ptu_count
        .is_some_and(|value| !matches!(value, Scalar::Null));
    let has_rate = model_info
        .cost_per_ptu_per_hour
        .is_some_and(|value| !matches!(value, Scalar::Null));
    if !has_count && !has_rate {
        return None;
    }
    if has_count != has_rate {
        return Some(UNPAIRED_COUNT_RATE);
    }
    let start = model_info
        .ptu_effective_from
        .and_then(PtuInstantSource::as_utc);
    if start.is_none() {
        return Some(MISSING_START);
    }
    if team_id_text(model_info.team_id).is_none() {
        return Some(MISSING_TEAM);
    }
    None
}

#[must_use]
pub fn declares_ptu(model_info: &PtuModelInfo<'_>) -> bool {
    let count = model_info
        .ptu_count
        .is_some_and(|value| !matches!(value, Scalar::Null));
    let rate = model_info
        .cost_per_ptu_per_hour
        .is_some_and(|value| !matches!(value, Scalar::Null));
    let start = model_info
        .ptu_effective_from
        .is_some_and(|source| !matches!(source, PtuInstantSource::Scalar(Scalar::Null)));
    let end = model_info
        .ptu_effective_to
        .is_some_and(|source| !matches!(source, PtuInstantSource::Scalar(Scalar::Null)));
    count || rate || start || end
}

#[must_use]
pub fn ptu_identity_error(
    declared_id: Option<&str>,
    taken: bool,
    current_id: Option<&str>,
    model_name: Option<&str>,
) -> Option<String> {
    let candidate = declared_id.filter(|id| !id.is_empty());
    let Some(unique_id) = candidate else {
        let suggestion = current_id
            .filter(|id| !id.is_empty())
            .unwrap_or("shown by GET /model/info");
        let reason = format!(
            "model_info.id is required when PTU fields are set. Without one the deployment is \
             identified by a hash of its litellm_params, so rotating a credential bills the \
             reservation a second time under the new identity. Set it to the id this deployment \
             already uses, {suggestion}, so the flat cost already written stays under one identity; \
             any other value starts a second one"
        );
        return Some(named(&reason, model_name));
    };
    if taken {
        let reason = format!(
            "model_info.id '{unique_id}' is declared on more than one deployment. Each would key \
             the same flat-cost row, so one reservation would go unbilled"
        );
        return Some(named(&reason, model_name));
    }
    None
}

fn team_id_text(team_id: Option<&Scalar>) -> Option<&str> {
    match team_id? {
        Scalar::Text(text) => (!text.is_empty()).then_some(text.as_str()),
        Scalar::Bool(_)
        | Scalar::Null
        | Scalar::Number(_)
        | Scalar::Array(_)
        | Scalar::Object(_) => None,
    }
}

fn named(reason: &str, model_name: Option<&str>) -> String {
    model_name.map_or_else(
        || reason.to_owned(),
        |name| format!("PTU configuration on model '{name}' is invalid: {reason}"),
    )
}
