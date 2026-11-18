#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, LookupClock};
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;

use crate::auth_read::table::{
    AuthTable, Create, FindMany, FindOne, Presence, ReadContext, Update,
};

pub(super) type Fields = BTreeMap<String, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct JwtMapping {
    pub(super) token: String,
    pub(super) jwt_issuer: String,
    pub(super) jwt_claim_name: String,
    pub(super) jwt_claim_value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OrganizationMembershipView {
    pub(super) organization_id: String,
    pub(super) user_role: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BudgetRelation {
    pub(super) max_budget: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RichRow {
    pub(super) mapping: Fields,
    pub(super) ordered: Vec<(String, String)>,
    pub(super) user_id: String,
    pub(super) organization_memberships: Option<Vec<OrganizationMembershipView>>,
    pub(super) jwt: JwtMapping,
    pub(super) vector_stores: Option<Vec<String>>,
    pub(super) tag_name: String,
    pub(super) access_group_name: String,
    pub(super) spend: Option<Decimal>,
    pub(super) budget: Option<BudgetRelation>,
}

pub(super) fn rich_row(user_id: &str) -> RichRow {
    RichRow {
        mapping: BTreeMap::from([
            ("id".to_owned(), "row-1".to_owned()),
            ("team_id".to_owned(), "team-1".to_owned()),
        ]),
        ordered: vec![
            ("team_id".to_owned(), "team-1".to_owned()),
            ("user_id".to_owned(), user_id.to_owned()),
            ("team_id".to_owned(), "team-1".to_owned()),
        ],
        user_id: user_id.to_owned(),
        organization_memberships: Some(vec![
            OrganizationMembershipView {
                organization_id: "org-2".to_owned(),
                user_role: None,
            },
            OrganizationMembershipView {
                organization_id: "org-1".to_owned(),
                user_role: Some("admin".to_owned()),
            },
        ]),
        jwt: JwtMapping {
            token: "jwt-token".to_owned(),
            jwt_issuer: "https://issuer.example".to_owned(),
            jwt_claim_name: "role".to_owned(),
            jwt_claim_value: "admin".to_owned(),
        },
        vector_stores: Some(vec!["store-2".to_owned(), "store-1".to_owned()]),
        tag_name: "prod".to_owned(),
        access_group_name: "group-a".to_owned(),
        spend: Some(Decimal::new(125, 2)),
        budget: Some(BudgetRelation { max_budget: None }),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RecordedCall {
    FindUnique {
        filter: Fields,
        include: Presence<Fields>,
    },
    FindFirst {
        filter: Fields,
        include: Presence<Fields>,
    },
    FindMany {
        filter: Presence<Fields>,
        include: Presence<Fields>,
        take: Presence<i64>,
    },
    Update {
        filter: Fields,
        data: Fields,
    },
    Create {
        data: Fields,
        include: Presence<Fields>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Mode {
    #[default]
    Row,
    Missing,
    Error,
    Pending,
    Delayed,
}

pub(super) struct RecordingRepo {
    mode: Mode,
    row: RichRow,
    rows: Vec<RichRow>,
    calls: Mutex<Vec<RecordedCall>>,
    dropped: AtomicBool,
}

impl RecordingRepo {
    pub(super) fn calls(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn dropped(&self) -> bool {
        self.dropped.load(Ordering::SeqCst)
    }

    fn record(&self, call: RecordedCall) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    async fn stall<Output>(&self) -> Result<Output, DbLookupError> {
        let _held = HeldFlag(&self.dropped);
        std::future::pending().await
    }

    async fn reply<Output>(&self, ready: Output) -> Result<Output, DbLookupError> {
        match self.mode {
            Mode::Pending => self.stall().await,
            Mode::Delayed => {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok(ready)
            }
            Mode::Error => Err(read_error()),
            Mode::Row | Mode::Missing => Ok(ready),
        }
    }

    async fn reply_one(&self) -> Result<Option<RichRow>, DbLookupError> {
        let ready = match self.mode {
            Mode::Missing => None,
            Mode::Row | Mode::Error | Mode::Pending | Mode::Delayed => Some(self.row.clone()),
        };
        self.reply(ready).await
    }

    async fn reply_many(&self) -> Result<Vec<RichRow>, DbLookupError> {
        let ready = match self.mode {
            Mode::Missing => Vec::new(),
            Mode::Row | Mode::Error | Mode::Pending | Mode::Delayed => self.rows.clone(),
        };
        self.reply(ready).await
    }

    async fn reply_created(&self) -> Result<RichRow, DbLookupError> {
        self.reply(self.row.clone()).await
    }
}

struct HeldFlag<'flag>(&'flag AtomicBool);

impl Drop for HeldFlag<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl AuthTable for RecordingRepo {
    type Row = RichRow;
    type Filter = Fields;
    type Include = Fields;
    type Data = Fields;

    async fn find_unique(
        &self,
        request: FindOne<Self::Filter, Self::Include>,
    ) -> Result<Option<Self::Row>, DbLookupError> {
        self.record(RecordedCall::FindUnique {
            filter: request.filter,
            include: request.include,
        });
        self.reply_one().await
    }

    async fn find_first(
        &self,
        request: FindOne<Self::Filter, Self::Include>,
    ) -> Result<Option<Self::Row>, DbLookupError> {
        self.record(RecordedCall::FindFirst {
            filter: request.filter,
            include: request.include,
        });
        self.reply_one().await
    }

    async fn find_many(
        &self,
        request: FindMany<Self::Filter, Self::Include>,
    ) -> Result<Vec<Self::Row>, DbLookupError> {
        self.record(RecordedCall::FindMany {
            filter: request.filter,
            include: request.include,
            take: request.take,
        });
        self.reply_many().await
    }

    async fn update(
        &self,
        request: Update<Self::Filter, Self::Data>,
    ) -> Result<Option<Self::Row>, DbLookupError> {
        self.record(RecordedCall::Update {
            filter: request.filter,
            data: request.data,
        });
        self.reply_one().await
    }

    async fn create(
        &self,
        request: Create<Self::Data, Self::Include>,
    ) -> Result<Self::Row, DbLookupError> {
        self.record(RecordedCall::Create {
            data: request.data,
            include: request.include,
        });
        self.reply_created().await
    }
}

pub(super) fn recording_repo(mode: Mode) -> Arc<RecordingRepo> {
    recording_repo_with(
        mode,
        vec![rich_row("second"), rich_row("first"), rich_row("second")],
    )
}

pub(super) fn recording_repo_with(mode: Mode, rows: Vec<RichRow>) -> Arc<RecordingRepo> {
    Arc::new(RecordingRepo {
        mode,
        row: rich_row("user-1"),
        rows,
        calls: Mutex::new(Vec::new()),
        dropped: AtomicBool::new(false),
    })
}

pub(super) fn recording_row_repo(mode: Mode, row: RichRow) -> Arc<RecordingRepo> {
    Arc::new(RecordingRepo {
        mode,
        row,
        rows: Vec::new(),
        calls: Mutex::new(Vec::new()),
        dropped: AtomicBool::new(false),
    })
}

pub(super) fn read_error() -> DbLookupError {
    DbLookupError::Database(DbError {
        error_type: ErrorType::Read,
        message: "the membership read failed".to_owned(),
        code: Some("XX000".to_owned()),
        meta_code: None,
        traceback_modules: vec!["asyncpg.protocol".to_owned()],
        cause: Some(Box::new(DbError {
            error_type: ErrorType::EngineConnection,
            message: "the pool refused the connection".to_owned(),
            code: None,
            meta_code: None,
            traceback_modules: Vec::new(),
            cause: None,
        })),
    })
}

pub(super) fn assert_deadline(error: DbLookupError, lookup: &str, seconds: f64) {
    let DbLookupError::Deadline(exceeded) = error else {
        panic!("the read must fail with a deadline, got {error:?}");
    };
    assert_eq!(exceeded.lookup, lookup);
    assert_eq!(exceeded.deadline_seconds.to_bits(), seconds.to_bits());
}

#[derive(Clone)]
pub(super) struct ManualClock(Arc<Mutex<Instant>>);

impl Default for ManualClock {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Instant::now())))
    }
}

impl ManualClock {
    pub(super) fn advance(&self, seconds: u64) {
        let mut now = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *now += Duration::from_secs(seconds);
    }
}

impl LookupClock for ManualClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(super) struct Harness {
    pub(super) clock: ManualClock,
    pub(super) tracker: Arc<DbLookupStallTracker<ManualClock>>,
    pub(super) tasks: TaskTracker,
}

impl Harness {
    pub(super) fn new() -> Self {
        let clock = ManualClock::default();
        let tracker = Arc::new(DbLookupStallTracker {
            clock: clock.clone(),
            last_hit: Mutex::new(None),
        });
        Self {
            clock,
            tracker,
            tasks: TaskTracker::new(),
        }
    }

    pub(super) fn context(&self, default_deadline_seconds: f64) -> ReadContext<ManualClock> {
        ReadContext {
            default_deadline_seconds,
            tracker: Arc::clone(&self.tracker),
            tasks: self.tasks.clone(),
        }
    }

    pub(super) async fn drain(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }
}

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(super) fn capture() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_max_level(Level::TRACE)
            .with_writer(logs.clone())
            .finish();
        (logs, tracing::subscriber::set_default(subscriber))
    }

    pub(super) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("the captured logs are UTF-8")
    }
}

impl io::Write for Logs {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the built-in secret patterns compile")
}
