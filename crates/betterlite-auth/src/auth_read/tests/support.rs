#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::error::Error;
use std::fmt::Debug;
use std::future::pending;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, LookupClock};
use betterlite_db::postgres_store::errors::DbError as RepositoryFault;
use betterlite_db::postgres_store::rows::KeyColumn;
use betterlite_db::user_repository::RowDecodeError;
use betterlite_db::user_repository::projection::UserRecordError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rust_decimal::Decimal;
use tokio::time::error::Elapsed;
use tokio::time::{sleep, timeout};
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::auth_read::repository::NativeKey;
use crate::auth_read::table::{
    AuthReadError, AuthTable, Create, FindMany, FindOne, ReadContext, Update,
};

pub(super) type Fields = BTreeMap<String, String>;

type OneRequest = FindOne<Fields, Fields>;
type ManyRequest = FindMany<Fields, Fields>;
type WriteRequest = Update<Fields, Fields>;
type OneAnswer = Result<Option<RichRow>, AuthReadError<RowDecodeError>>;
type ManyAnswer = Result<Vec<RichRow>, AuthReadError<RowDecodeError>>;
type RowAnswer = Result<RichRow, AuthReadError<RowDecodeError>>;

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
    FindUnique(OneRequest),
    FindFirst(OneRequest),
    FindMany(ManyRequest),
    Update(WriteRequest),
    Create(Create<Fields, Fields>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Mode {
    #[default]
    Row,
    Error,
    Elapsed,
    Pending,
    Delayed,
}

async fn elapsed() -> Elapsed {
    timeout(Duration::from_secs(1), pending::<()>())
        .await
        .expect_err("the pending lookup elapses")
}

pub(super) struct RecordingRepo {
    mode: Mode,
    row: RichRow,
    rows: Vec<RichRow>,
    calls: Mutex<Vec<RecordedCall>>,
    dropped: AtomicBool,
}

impl RecordingRepo {
    fn new(mode: Mode, row: RichRow, rows: Vec<RichRow>) -> Self {
        let calls = Mutex::new(Vec::new());
        let dropped = AtomicBool::new(false);
        Self {
            mode,
            row,
            rows,
            calls,
            dropped,
        }
    }

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

    async fn stall<Output>(&self) -> Result<Output, AuthReadError<RowDecodeError>> {
        let _held = HeldFlag(&self.dropped);
        pending().await
    }

    async fn reply<Output>(&self, ready: Output) -> Result<Output, AuthReadError<RowDecodeError>> {
        match self.mode {
            Mode::Pending => self.stall().await,
            Mode::Delayed => {
                sleep(Duration::from_secs(60)).await;
                Ok(ready)
            }
            Mode::Error => Err(repository_error()),
            Mode::Elapsed => Err(AuthReadError::Lookup(DbLookupError::Elapsed(
                elapsed().await,
            ))),
            Mode::Row => Ok(ready),
        }
    }

    async fn reply_one(&self) -> Result<Option<RichRow>, AuthReadError<RowDecodeError>> {
        self.reply(Some(self.row.clone())).await
    }

    async fn reply_many(&self) -> Result<Vec<RichRow>, AuthReadError<RowDecodeError>> {
        self.reply(self.rows.clone()).await
    }

    async fn reply_created(&self) -> Result<RichRow, AuthReadError<RowDecodeError>> {
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
    type Key = Fields;
    type Filter = Fields;
    type Include = Fields;
    type Data = Fields;
    type Failure = RowDecodeError;

    async fn find_unique(&self, request: OneRequest) -> OneAnswer {
        self.record(RecordedCall::FindUnique(request));
        self.reply_one().await
    }

    async fn find_first(&self, request: OneRequest) -> OneAnswer {
        self.record(RecordedCall::FindFirst(request));
        self.reply_one().await
    }

    async fn find_many(&self, request: ManyRequest) -> ManyAnswer {
        self.record(RecordedCall::FindMany(request));
        self.reply_many().await
    }

    async fn update(&self, request: WriteRequest) -> OneAnswer {
        self.record(RecordedCall::Update(request));
        self.reply_one().await
    }

    async fn create(&self, request: Create<Fields, Fields>) -> RowAnswer {
        self.record(RecordedCall::Create(request));
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
    Arc::new(RecordingRepo::new(mode, rich_row("user-1"), rows))
}

pub(super) fn recording_row_repo(mode: Mode, row: RichRow) -> Arc<RecordingRepo> {
    Arc::new(RecordingRepo::new(mode, row, Vec::new()))
}

pub(super) fn native_key() -> NativeKey<Infallible> {
    NativeKey {
        column: KeyColumn::Id,
        value: Box::new("user-1"),
    }
}

pub(super) fn assert_connect<Output, Failure>(read: Result<Output, AuthReadError<Failure>>)
where
    Failure: Error + Debug + 'static,
{
    match read {
        Err(AuthReadError::Repository(RepositoryFault::NoDbConnected)) => {}
        Err(other) => panic!("the native failure is not a deadline, got {other:?}"),
        Ok(_) => panic!("the unconnected repository returns no row"),
    }
}

pub(super) fn repository_error() -> AuthReadError<RowDecodeError> {
    AuthReadError::Repository(RepositoryFault::InvalidRecord {
        source: RowDecodeError::User {
            source: UserRecordError::NotAnObject,
        },
    })
}

pub(super) fn assert_deadline(error: AuthReadError<RowDecodeError>, lookup: &str, seconds: f64) {
    let AuthReadError::Lookup(DbLookupError::Deadline(exceeded)) = error else {
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
        *now = now
            .checked_add(Duration::from_secs(seconds))
            .expect("the manual clock stays inside the instant range");
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
        (logs, set_default(subscriber))
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

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the built-in secret patterns compile")
}
