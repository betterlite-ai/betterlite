use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use indexmap::IndexMap;
use indexmap::map::Entry;
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use serde::Serialize;
use serde_with::{FromInto, serde_as};

use super::rows::TeamSpend;

#[derive(Serialize)]
#[serde(transparent)]
struct JsonAmount(#[serde(with = "rust_decimal::serde::arbitrary_precision")] Decimal);

impl From<Decimal> for JsonAmount {
    fn from(amount: Decimal) -> Self {
        Self(amount)
    }
}

const TOP_TEAMS: usize = 10;
const UNASSIGNED: &str = "Unassigned";

#[derive(Debug, thiserror::Error)]
pub(super) enum TeamTotalsError {
    #[error("the database returned a null team spend total")]
    NullTotal,
    #[error("summing the team spend totals overflowed Decimal's range")]
    Overflow,
}

#[derive(Serialize)]
pub(super) struct TeamTotalsBody {
    pub(super) daily_spend: Vec<TeamDailyRow>,
    pub(super) teams: BTreeSet<String>,
    pub(super) total_spend_per_team: Vec<TeamTotalRow>,
}

#[serde_as]
#[derive(Serialize)]
pub(super) struct TeamDailyRow {
    pub(super) date: NaiveDate,
    #[serde(flatten)]
    #[serde_as(serialize_as = "BTreeMap<_, FromInto<JsonAmount>>")]
    pub(super) teams: BTreeMap<String, Decimal>,
}

#[derive(Serialize)]
pub(super) struct TeamTotalRow {
    pub(super) team_id: String,
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision::serialize")]
    pub(super) total_spend: Decimal,
}

#[derive(Default)]
struct TeamTotals {
    daily: BTreeMap<NaiveDate, BTreeMap<String, Decimal>>,
    total_by_team: IndexMap<String, Decimal>,
    teams: BTreeSet<String>,
}

impl TeamTotals {
    fn add(&mut self, row: &TeamSpend) -> Result<(), TeamTotalsError> {
        let Some(date) = row.spend_date else {
            return Ok(());
        };
        let Some(amount) = row.total_spend else {
            return Err(TeamTotalsError::NullTotal);
        };
        let alias = row
            .team_alias
            .clone()
            .unwrap_or_else(|| UNASSIGNED.to_owned());
        let rounded = amount.round_dp_with_strategy(2, RoundingStrategy::MidpointNearestEven);
        self.daily
            .entry(date)
            .or_default()
            .insert(alias.clone(), rounded);
        match self.total_by_team.entry(alias.clone()) {
            Entry::Occupied(mut entry) => {
                let total = entry.get_mut();
                *total = total
                    .checked_add(rounded)
                    .ok_or(TeamTotalsError::Overflow)?;
            }
            Entry::Vacant(entry) => {
                entry.insert(rounded);
            }
        }
        self.teams.insert(alias);
        Ok(())
    }

    fn into_body(self) -> TeamTotalsBody {
        let mut ranked: Vec<(String, Decimal)> = self.total_by_team.into_iter().collect();
        ranked.sort_by(|left, right| right.1.cmp(&left.1));
        let total_spend_per_team = ranked
            .into_iter()
            .take(TOP_TEAMS)
            .map(|(team_id, total_spend)| TeamTotalRow {
                team_id,
                total_spend,
            })
            .collect();
        let daily_spend = self
            .daily
            .into_iter()
            .map(|(date, teams)| TeamDailyRow { date, teams })
            .collect();
        TeamTotalsBody {
            daily_spend,
            teams: self.teams,
            total_spend_per_team,
        }
    }
}

pub(super) fn team_totals(rows: &[TeamSpend]) -> Result<TeamTotalsBody, TeamTotalsError> {
    let mut totals = TeamTotals::default();
    for row in rows {
        totals.add(row)?;
    }
    Ok(totals.into_body())
}
