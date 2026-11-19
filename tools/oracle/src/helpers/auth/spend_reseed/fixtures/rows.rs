use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_auth::spend_reseed::rows::{
    SpendAggregateRow, SpendCounterRows, SpendRow, SpendRowLookup, WindowEntity, WindowSpendRow,
};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_oracle_helper_api::error::HelperError;
use chrono::{DateTime, Utc};
use serde_json::Number;

use super::fakes::{FakeError, timestamp};
use super::input::Input;

fn stored(amount: f64) -> Option<JsonData> {
    Number::from_f64(amount).map(JsonData::Number)
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
        let window = input
            .window
            .as_ref()
            .map(|(start, spend)| timestamp(start).map(|moment| (moment, *spend)))
            .transpose()?;
        let logs = input
            .logs
            .iter()
            .map(|(start, spend)| timestamp(start).map(|moment| (moment, *spend)))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            row: input.row_spend,
            window,
            logs,
            fails: input.fails.0,
        })
    }
}

impl SpendCounterRows for FakeRows {
    type Error = FakeError;

    fn spend_row(
        &self,
        _lookup: SpendRowLookup<'_>,
    ) -> impl Future<Output = Result<Option<SpendRow>, FakeError>> + Send {
        if self.fails {
            return ready(Err(FakeError));
        }
        ready(Ok(self.row.map(|amount| SpendRow {
            spend: stored(amount),
        })))
    }

    fn window_row(
        &self,
        _entity: WindowEntity,
        _entity_id: &str,
        _window_duration: &str,
    ) -> impl Future<Output = Result<Option<WindowSpendRow>, FakeError>> + Send {
        if self.fails {
            return ready(Err(FakeError));
        }
        ready(Ok(self.window.map(|(start, spend)| WindowSpendRow {
            window_start: start,
            spend: stored(spend),
        })))
    }

    fn window_log_rows(
        &self,
        _entity: WindowEntity,
        _entity_id: &str,
        window_start: DateTime<Utc>,
    ) -> impl Future<Output = Result<Vec<SpendAggregateRow>, FakeError>> + Send {
        if self.fails {
            return ready(Err(FakeError));
        }
        let matching: Vec<f64> = self
            .logs
            .iter()
            .filter(|(start, _spend)| start.utc() >= window_start)
            .map(|(_start, spend)| *spend)
            .collect();
        if matching.is_empty() {
            return ready(Ok(Vec::new()));
        }
        let total: f64 = matching.iter().sum();
        ready(Ok(vec![aggregate(total)]))
    }
}
