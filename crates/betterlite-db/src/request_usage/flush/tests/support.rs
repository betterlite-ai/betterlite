use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::tool_spend::ToolUsageTransaction;
use chrono::{DateTime, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use tokio::sync::{Mutex as AsyncMutex, Notify};
use tokio_util::sync::CancellationToken;

use crate::autorouter_drain::AutoRouterDrain;
use crate::autorouter_turn_sql::transaction::{AutoRouterTurnTransaction, TurnFlag, TurnTime};
use crate::baseline::flush::queue::BaselineQueue;
use crate::baseline::records::{BaselineAccountingRecord, decode_baseline_record};
use crate::guardrail_usage_rollups::SharedPendingRollups;
use crate::guardrail_usage_rollups::pending::PendingRollups;
use crate::model_usage::{ModelUsageKey, ModelUsageTransaction};
use crate::redis_spend::buffer::RedisUpdateBuffer;
use crate::redis_spend::events::NoRedisBufferEventSink;
use crate::redis_spend::log_row::{SpendLogRow, SpendLogValue};
use crate::request_usage::flush::{
    GuardrailStage, ModelUsageDrain, RequestUsageQueues, SpendLogsJob, ToolUsageDrain,
};
use crate::request_usage::queues::UsageQueue;
use crate::shadow_tables::funnel::FunnelQueue;
use crate::spend_batches::StatementLimits;
use crate::spend_log_storage::rows::SpendLogQueue;
use crate::spend_log_storage::rows::writer::{FlushPolicy, NoSpendLogFailureSink, SpendLogFlush};
use crate::spend_queues::entity::SpendAmount;
use crate::tool_spend::FlushPolicy as ToolFlushPolicy;

use super::fakes::{
    FakeAccounting, FakeGuardrailSink, FakeLogWriter, FakeModelWriter, FakeShadowStore,
    FakeToolWriter, FakeTurnWriter, NoClient, NoJitter, RecordingPause, SteppingClock,
};

const LIMITS: StatementLimits = StatementLimits {
    max_bytes: 1_000_000,
    max_rows: 1_000,
};

// One canonical baseline record B1699 stores, for a backlog the pass must flush.
const TEMPLATE_RECORD: &str = r#"{"scope":"autorouter-baseline:v3:0000000000000000000000000000000000000000000000000000000000000000","api_key":"sk-1","session_id":"sess-1","router_name":"auto-router","baseline_model":"anthropic/claude-opus-5","observation":{"request_id":"req-template","started_at":1.5,"available_at":2.0,"outcome":"complete","baseline_equivalent":true},"pricing":{"model":"m","provider":"anthropic","prices":null,"basis":[null,null,null],"actual_spend":0.5,"classifier_cost":0.0},"turn":null,"daily":null}"#;

// The events the pass ran, in the order the fakes saw them.
#[derive(Clone, Default)]
pub(super) struct Trace {
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl Trace {
    pub(super) fn record(&self, event: &'static str) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    pub(super) fn events(&self) -> Vec<&'static str> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    // The stage order, with the repeated rows of one stage folded into one entry.
    pub(super) fn stage_order(&self) -> Vec<&'static str> {
        let mut ordered: Vec<&'static str> = Vec::new();
        for event in self.events() {
            if ordered.last() != Some(&event) {
                ordered.push(event);
            }
        }
        ordered
    }

    pub(super) fn ran(&self, event: &'static str) -> bool {
        self.events().contains(&event)
    }
}

pub(super) type HarnessJob<'run> = SpendLogsJob<
    'run,
    FakeLogWriter,
    NoClient,
    NoSpendLogFailureSink,
    FakeGuardrailSink,
    FakeToolWriter,
    FakeModelWriter,
    FakeTurnWriter,
    FakeAccounting,
    FakeShadowStore,
>;

// Every queue, sink and lock one job pass borrows.
pub(super) struct Harness {
    pub(super) trace: Trace,
    pub(super) logs: SpendLogQueue,
    pub(super) tool_usage: UsageQueue<ToolUsageTransaction>,
    pub(super) model_usage: UsageQueue<ModelUsageTransaction>,
    pub(super) turns_queue: UsageQueue<AutoRouterTurnTransaction>,
    pub(super) baseline: BaselineQueue,
    pub(super) funnel: FunnelQueue,
    pub(super) log_writer: FakeLogWriter,
    pub(super) redis: RedisUpdateBuffer,
    pub(super) failures: NoSpendLogFailureSink,
    pub(super) pause: RecordingPause,
    pub(super) clock: SteppingClock,
    pub(super) pending: SharedPendingRollups,
    pub(super) guardrail_sink: FakeGuardrailSink,
    pub(super) tool_writer: FakeToolWriter,
    pub(super) model_writer: FakeModelWriter,
    pub(super) turn_writer: FakeTurnWriter,
    pub(super) accounting: FakeAccounting,
    pub(super) notifier: Notify,
    pub(super) shadows: FakeShadowStore,
    pub(super) write_lock: AsyncMutex<()>,
    pub(super) shutdown: CancellationToken,
    pub(super) jitter: NoJitter,
}

impl Harness {
    pub(super) fn new() -> Self {
        let trace = Trace::default();
        Self {
            trace: trace.clone(),
            logs: SpendLogQueue::new(8_388_608),
            tool_usage: UsageQueue::new(20_000).unwrap(),
            model_usage: UsageQueue::new(20_000).unwrap(),
            turns_queue: UsageQueue::new(20_000).unwrap(),
            baseline: BaselineQueue::new(20_000).unwrap(),
            funnel: FunnelQueue::default(),
            log_writer: FakeLogWriter::new(trace.clone()),
            redis: RedisUpdateBuffer {
                lists: None,
                enabled: false,
                dequeue_count: 1_000,
                events: Arc::new(NoRedisBufferEventSink),
            },
            failures: NoSpendLogFailureSink,
            pause: RecordingPause,
            clock: SteppingClock::new(),
            pending: Arc::new(Mutex::new(PendingRollups::default())),
            guardrail_sink: FakeGuardrailSink::new(trace.clone()),
            tool_writer: FakeToolWriter::new(trace.clone()),
            model_writer: FakeModelWriter::new(trace.clone()),
            turn_writer: FakeTurnWriter::new(trace.clone()),
            accounting: FakeAccounting::new(trace.clone()),
            notifier: Notify::new(),
            shadows: FakeShadowStore::new(trace),
            write_lock: AsyncMutex::new(()),
            shutdown: CancellationToken::new(),
            jitter: NoJitter,
        }
    }

    pub(super) fn job(&self) -> HarnessJob<'_> {
        SpendLogsJob {
            queues: self.queues(),
            logs: self.log_flush(),
            guardrails: self.guardrails(),
            tools: self.tools(),
            models: self.models(),
            turns: AutoRouterDrain::new(&self.turn_writer, &self.jitter, &self.pause),
            accounting: &self.accounting,
            notifier: &self.notifier,
            shadows: &self.shadows,
            write_lock: &self.write_lock,
            shutdown: &self.shutdown,
        }
    }

    fn queues(&self) -> RequestUsageQueues<'_> {
        RequestUsageQueues {
            logs: &self.logs,
            tool_usage: &self.tool_usage,
            model_usage: &self.model_usage,
            turns: &self.turns_queue,
            baseline: &self.baseline,
            funnel: &self.funnel,
        }
    }

    fn log_flush(&self) -> SpendLogFlush<'_, FakeLogWriter, NoClient, NoSpendLogFailureSink> {
        SpendLogFlush {
            writer: &self.log_writer,
            external: None,
            queue: &self.logs,
            redis: &self.redis,
            failures: &self.failures,
            pause: &self.pause,
            clock: &self.clock,
            policy: FlushPolicy {
                retries: 0,
                limits: LIMITS,
                spend_logs_url: None,
            },
        }
    }

    fn guardrails(&self) -> GuardrailStage<'_, FakeGuardrailSink> {
        GuardrailStage {
            pending: &self.pending,
            sink: &self.guardrail_sink,
            limits: LIMITS,
            pause: &self.pause,
        }
    }

    fn tools(&self) -> ToolUsageDrain<'_, FakeToolWriter> {
        ToolUsageDrain {
            writer: &self.tool_writer,
            policy: ToolFlushPolicy {
                limits: LIMITS,
                retries: 0,
            },
            jitter: &self.jitter,
            pause: &self.pause,
        }
    }

    fn models(&self) -> ModelUsageDrain<'_, FakeModelWriter> {
        ModelUsageDrain {
            writer: &self.model_writer,
            retries: 0,
            jitter: &self.jitter,
            pause: &self.pause,
        }
    }
}

pub(super) fn text(value: &str) -> SpendLogValue {
    SpendLogValue::Text(value.to_owned())
}

pub(super) const fn amount(value: Decimal) -> SpendLogValue {
    SpendLogValue::Decimal(value)
}

fn guardrail_metadata() -> SpendLogValue {
    SpendLogValue::Object(BTreeMap::from([(
        "guardrail_information".to_owned(),
        SpendLogValue::Array(vec![SpendLogValue::Object(BTreeMap::from([
            ("guardrail_id".to_owned(), text("g1")),
            ("guardrail_status".to_owned(), text("success")),
        ]))]),
    )]))
}

// A spend log row with one successful guardrail outcome and a start time the usage tracker can read.
pub(super) fn guardrail_row(request_id: &str) -> SpendLogRow {
    let mut row = BTreeMap::new();
    row.insert("request_id".to_owned(), text(request_id));
    row.insert("startTime".to_owned(), text("2026-01-02T03:04:05"));
    row.insert("api_key".to_owned(), text("sk-1"));
    row.insert("team_id".to_owned(), text("t1"));
    row.insert("spend".to_owned(), amount(Decimal::new(25, 2)));
    row.insert("metadata".to_owned(), guardrail_metadata());
    row
}

// The same row with a start time the guardrail payload cannot read.
pub(super) fn unreadable_guardrail_row(request_id: &str) -> SpendLogRow {
    BTreeMap::from([
        ("request_id".to_owned(), text(request_id)),
        ("startTime".to_owned(), SpendLogValue::Whole(5)),
        ("metadata".to_owned(), guardrail_metadata()),
    ])
}

pub(super) fn tool_transaction(request_id: &str) -> ToolUsageTransaction {
    ToolUsageTransaction {
        request_id: request_id.to_owned(),
        date: "2026-01-02".to_owned(),
        start_time: moment(),
        tool_names: vec!["search".to_owned()],
        spend: Decimal::new(25, 2),
        total_tokens: 10,
    }
}

pub(super) fn model_transaction() -> ModelUsageTransaction {
    let key = ModelUsageKey {
        date: "2026-01-02".to_owned(),
        model_group: "gpt-4o".to_owned(),
        model: "gpt-4o".to_owned(),
        custom_llm_provider: "openai".to_owned(),
        task_type: "uncategorized".to_owned(),
    };
    ModelUsageTransaction {
        key,
        spend: SpendAmount(Decimal::new(25, 2)),
        prompt_tokens: 3,
        completion_tokens: 4,
        successful: true,
    }
}

pub(super) fn turn_transaction(session_id: &str) -> AutoRouterTurnTransaction {
    AutoRouterTurnTransaction {
        api_key: "sk-1".to_owned(),
        session_id: session_id.to_owned(),
        router_name: "auto-router".to_owned(),
        router_type: "simple-shuffle".to_owned(),
        model: "gpt-4o".to_owned(),
        turn_at: TurnTime::Naive(naive_moment()),
        total_tokens: 7,
        spend: Decimal::new(25, 2),
        saved_spend: Decimal::ZERO,
        classifier_cost: Decimal::ZERO,
        covered: TurnFlag(false),
        cache_hit: TurnFlag(false),
        cache_ttl_seconds: None,
        cache_touched: TurnFlag(false),
        tier: None,
        baseline_model: None,
        savings_estimated_turns: 0,
        savings_estimated_actual_spend: Decimal::ZERO,
        savings_estimated_saved_spend: Decimal::ZERO,
        user_id: "u1".to_owned(),
    }
}

// Fill a request-time queue with the count records the case builds, failing the test if the queue rejects one.
pub(super) async fn fill_queue<T, F>(queue: &UsageQueue<T>, count: u32, make: F)
where
    F: Fn(u32) -> T,
{
    for index in 0..count {
        queue.push(make(index)).await.unwrap();
    }
}

pub(super) fn baseline_record() -> BaselineAccountingRecord {
    decode_baseline_record(TEMPLATE_RECORD.as_bytes()).unwrap()
}

fn naive_moment() -> NaiveDateTime {
    NaiveDateTime::parse_from_str("2026-01-02T03:04:05", "%Y-%m-%dT%H:%M:%S").unwrap()
}

fn moment() -> DateTime<Utc> {
    naive_moment().and_utc()
}
