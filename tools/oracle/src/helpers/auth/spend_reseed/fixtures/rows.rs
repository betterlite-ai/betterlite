use std::collections::BTreeMap;

use betterlite_auth::spend_reseed::rows::{
    SpendAggregateRow, SpendCounterRows, SpendRow, SpendRowLookup, WindowEntity, WindowSpendRow,
};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_oracle_helper_api::error::HelperError;
use chrono::{DateTime, Utc};

use super::fakes::{FakeError, timestamp};
use super::input::Input;

fn stored(amount: f64) -> Option<JsonData> {
    serde_json::Number::from_f64(amount).map(JsonData::Number)
}

fn aggregate(total: f64) -> SpendAggregateRow {
    let spend = stored(total).unwrap_or(JsonData::Null);
    SpendAggregateRow::Mapping(BTreeMap::from([(
        "_sum".to_owned(),
        JsonData::Object(BTreeMap::from([("spend".to_owned(), spend)])),
    )]))
}

pub(crate) struct FakeRows {
    row: Option<f64>,
    window: Option<(Timestamp, f64)>,
    logs: Vec<(Timestamp, f64)>,
    fails: bool,
}

impl FakeRows {
    pub(crate) fn build(input: &Input) -> Result<Self, Box<HelperError>> {
        let window = match &input.window {
            Some((start, spend)) => Some((timestamp(start)?, *spend)),
            None => None,
        };
        let mut logs = Vec::new();
        for (start, spend) in &input.logs {
            logs.push((timestamp(start)?, *spend));
        }
        Ok(Self {
            row: input.row_spend,
            window,
            logs,
            fails: input.fails,
        })
    }
}

impl SpendCounterRows for FakeRows {
    type Error = FakeError;

    async fn spend_row(&self, _lookup: SpendRowLookup<'_>) -> Result<Option<SpendRow>, FakeError> {
        if self.fails {
            return Err(FakeError);
        }
        Ok(self.row.map(|amount| SpendRow {
            spend: stored(amount),
        }))
    }

    async fn window_row(
        &self,
        _entity: WindowEntity,
        _entity_id: &str,
        _window_duration: &str,
    ) -> Result<Option<WindowSpendRow>, FakeError> {
        if self.fails {
            return Err(FakeError);
        }
        Ok(self.window.map(|(start, spend)| WindowSpendRow {
            window_start: start,
            spend: stored(spend),
        }))
    }

    async fn window_log_rows(
        &self,
        _entity: WindowEntity,
        _entity_id: &str,
        window_start: DateTime<Utc>,
    ) -> Result<Vec<SpendAggregateRow>, FakeError> {
        if self.fails {
            return Err(FakeError);
        }
        let mut matching = Vec::new();
        for (start, spend) in &self.logs {
            if start.utc() >= window_start {
                matching.push(*spend);
            }
        }
        if matching.is_empty() {
            return Ok(Vec::new());
        }
        let total: f64 = matching.iter().sum();
        Ok(vec![aggregate(total)])
    }
}
