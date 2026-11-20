use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex};

use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use crate::database_failures::gate::{DbLookupStallTracker, LoopBoundSemaphore, SystemClock};
use crate::spend_rows::key::{
    SpendCounterKey, WindowDurationValidity, WindowEntity, decode_spend_counter_key,
};
use crate::spend_rows::reader::SpendCounterReader;
use crate::spend_rows::rows::{SpendCounterRows, SpendRow, SpendRowLookup};

pub(crate) fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal amount")
}

pub(crate) struct FakeRows {
    rows: BTreeMap<String, Option<Decimal>>,
    fails: bool,
    calls: Mutex<Vec<String>>,
}

impl FakeRows {
    pub(crate) fn with_rows(rows: BTreeMap<String, Option<Decimal>>) -> Self {
        Self {
            rows,
            fails: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            rows: BTreeMap::new(),
            fails: true,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("the fake rows lock").clone()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake spend row read failed")]
pub(crate) struct FakeRowsError;

pub(crate) fn lookup_label(lookup: SpendRowLookup<'_>) -> String {
    match lookup {
        SpendRowLookup::Key(token) => format!("key:{token}"),
        SpendRowLookup::TeamMember { user_id, team_id } => {
            format!("team_member:{user_id}:{team_id}")
        }
        SpendRowLookup::Team(team_id) => format!("team:{team_id}"),
        SpendRowLookup::User(user_id) => format!("user:{user_id}"),
        SpendRowLookup::Organization(organization_id) => format!("org:{organization_id}"),
        SpendRowLookup::Project(project_id) => format!("project:{project_id}"),
        SpendRowLookup::EndUser(user_id) => format!("end_user:{user_id}"),
    }
}

impl SpendCounterRows for FakeRows {
    type Error = FakeRowsError;

    async fn spend_row(
        &self,
        lookup: SpendRowLookup<'_>,
    ) -> Result<Option<SpendRow>, FakeRowsError> {
        let label = lookup_label(lookup);
        self.calls
            .lock()
            .expect("the fake rows lock")
            .push(label.clone());
        if self.fails {
            return Err(FakeRowsError);
        }
        Ok(self
            .rows
            .get(&label)
            .copied()
            .map(|spend| SpendRow { spend }))
    }
}

pub(crate) struct HangingRows;

impl SpendCounterRows for HangingRows {
    type Error = FakeRowsError;

    async fn spend_row(
        &self,
        _lookup: SpendRowLookup<'_>,
    ) -> Result<Option<SpendRow>, FakeRowsError> {
        std::future::pending().await
    }
}

pub(crate) struct FixedDurations;

impl WindowDurationValidity for FixedDurations {
    fn is_valid(&self, spec: &str) -> bool {
        matches!(spec, "1h" | "1d" | "2d" | "30d")
    }
}

pub(crate) struct AnyDurations;

impl WindowDurationValidity for AnyDurations {
    fn is_valid(&self, _spec: &str) -> bool {
        true
    }
}

pub(crate) struct NoDurations;

impl WindowDurationValidity for NoDurations {
    fn is_valid(&self, _spec: &str) -> bool {
        false
    }
}

pub(crate) struct Services {
    pub(crate) tasks: TaskTracker,
    pub(crate) gate: Arc<LoopBoundSemaphore>,
    pub(crate) stall_tracker: Arc<DbLookupStallTracker>,
}

pub(crate) fn services(gate_slots: usize) -> Services {
    Services {
        tasks: TaskTracker::new(),
        gate: Arc::new(LoopBoundSemaphore {
            value: gate_slots,
            slot: Mutex::new(None),
        }),
        stall_tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
    }
}

pub(crate) fn reader_with<Rows: SpendCounterRows>(
    rows: Option<Arc<Rows>>,
    durations: Arc<dyn WindowDurationValidity>,
    services: Services,
    deadline_seconds: f64,
) -> SpendCounterReader<Rows> {
    SpendCounterReader {
        rows,
        durations,
        tasks: services.tasks,
        gate: services.gate,
        stall_tracker: services.stall_tracker,
        deadline_seconds,
    }
}

pub(crate) fn replay_corpus_key(bytes: &[u8]) -> String {
    let Some((&tag, rest)) = bytes.split_first() else {
        return "empty".to_owned();
    };
    let decoded = if (tag & 1) == 0 {
        decode_spend_counter_key(rest, &AnyDurations)
    } else {
        decode_spend_counter_key(rest, &NoDurations)
    };
    match decoded {
        Ok(key) => describe(key),
        Err(_error) => "error".to_owned(),
    }
}

fn describe(key: SpendCounterKey<'_>) -> String {
    match key {
        SpendCounterKey::Key(token) => format!("key:{token}"),
        SpendCounterKey::TeamMember { user_id, team_id } => {
            format!("team_member:{user_id}:{team_id}")
        }
        SpendCounterKey::Team(team_id) => format!("team:{team_id}"),
        SpendCounterKey::User(user_id) => format!("user:{user_id}"),
        SpendCounterKey::Organization(organization_id) => format!("org:{organization_id}"),
        SpendCounterKey::Project(project_id) => format!("project:{project_id}"),
        SpendCounterKey::EndUser(user_id) => format!("end_user:{user_id}"),
        SpendCounterKey::Window { entity, duration } => match entity {
            WindowEntity::Key => format!("window:key:{duration}"),
            WindowEntity::Team => format!("window:team:{duration}"),
        },
        SpendCounterKey::Unknown(text) => format!("unknown:{text}"),
    }
}
