use std::io;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};

use rust_decimal::Decimal;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;

use crate::daily_spend_sql::entities::SpendRow;
use crate::daily_spend_sql::values::SpendColumnValue;
use crate::spend_queues::daily::DailySpendUpdate;
use crate::spend_queues::entity::{EntityType, SpendAmount, SpendUpdateQueueItem};
use crate::spend_queues::events::{SpendQueueEvent, SpendQueueEventSink};

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .unwrap()
    }
}

impl io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

pub(super) struct LogCapture {
    logs: CapturedLogs,
    _guard: DefaultGuard,
}

impl LogCapture {
    pub(super) fn text(&self) -> String {
        self.logs.text()
    }

    pub(super) fn contains(&self, needle: &str) -> bool {
        self.text().contains(needle)
    }
}

pub(super) fn capture_logs() -> LogCapture {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(logs.clone())
        .finish();
    let guard = set_default(subscriber);
    LogCapture {
        logs,
        _guard: guard,
    }
}

#[derive(Default)]
pub(super) struct RecordingSink {
    events: Mutex<Vec<SpendQueueEvent>>,
}

impl RecordingSink {
    pub(super) fn events(&self) -> Vec<SpendQueueEvent> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SpendQueueEventSink for RecordingSink {
    fn emit(&self, event: SpendQueueEvent) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

pub(super) fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

pub(super) fn amount(text: &str) -> SpendAmount {
    SpendAmount(decimal(text))
}

pub(super) fn item(
    kind: Option<EntityType>,
    entity_id: Option<&str>,
    response_cost: Option<&str>,
) -> SpendUpdateQueueItem {
    SpendUpdateQueueItem {
        entity_type: kind,
        entity_id: entity_id.map(str::to_owned),
        response_cost: response_cost.map(amount),
    }
}

pub(super) fn daily_row(spend: &str) -> SpendRow {
    let fields = [
        ("date", SpendColumnValue::Text("2026-01-01".to_owned())),
        ("api_key", SpendColumnValue::Text("key".to_owned())),
        ("model_group", SpendColumnValue::Text("group".to_owned())),
        ("prompt_tokens", SpendColumnValue::Int(3)),
        ("completion_tokens", SpendColumnValue::Int(2)),
        ("api_requests", SpendColumnValue::Int(1)),
        ("successful_requests", SpendColumnValue::Int(1)),
        ("failed_requests", SpendColumnValue::Int(0)),
        ("spend", SpendColumnValue::Decimal(decimal(spend))),
    ];
    fields
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

pub(super) fn with_field(
    mut row: SpendRow,
    name: &'static str,
    value: SpendColumnValue,
) -> SpendRow {
    row.insert(name.to_owned(), value);
    row
}

pub(super) fn without_field(mut row: SpendRow, name: &'static str) -> SpendRow {
    row.remove(name);
    row
}

pub(super) fn update(key: &str, row: SpendRow) -> DailySpendUpdate {
    let mut update = DailySpendUpdate::new();
    update.insert(key.to_owned(), row);
    update
}
