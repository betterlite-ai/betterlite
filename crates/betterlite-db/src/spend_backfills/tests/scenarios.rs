use crate::spend_backfills::BackfillOutcome;
use crate::spend_backfills::fixtures::KeyTable;

use super::postgres::{
    BudgetPlan, ExpectedRow, FailureScenario, HistoryScenario, KeyPlan, LogPlan, Operation,
};

pub(super) const FLOOR_BUDGETS: [BudgetPlan<'_>; 1] = [BudgetPlan {
    budget_id: "budget-linked",
    budget_duration: Some("30d"),
}];

pub(super) const FLOOR_KEYS: [KeyPlan<'_>; 5] = [
    KeyPlan {
        table: KeyTable::Active,
        id: "key-own",
        token: "key-own",
        spend: (5, 0),
        total_spend: (1, 0),
        budget_duration: Some("1d"),
        budget_id: None,
    },
    KeyPlan {
        table: KeyTable::Active,
        id: "key-linked",
        token: "key-linked",
        spend: (5, 0),
        total_spend: (1, 0),
        budget_duration: None,
        budget_id: Some("budget-linked"),
    },
    KeyPlan {
        table: KeyTable::Active,
        id: "key-plain",
        token: "key-plain",
        spend: (5, 0),
        total_spend: (1, 0),
        budget_duration: None,
        budget_id: None,
    },
    KeyPlan {
        table: KeyTable::Archived,
        id: "archived-own",
        token: "archived-own-token",
        spend: (7, 0),
        total_spend: (2, 0),
        budget_duration: Some("1d"),
        budget_id: None,
    },
    KeyPlan {
        table: KeyTable::Archived,
        id: "archived-plain",
        token: "archived-plain-token",
        spend: (7, 0),
        total_spend: (2, 0),
        budget_duration: None,
        budget_id: None,
    },
];

pub(super) const FLOOR_TOTALS: [ExpectedRow<'_>; 5] = [
    ExpectedRow {
        table: KeyTable::Active,
        id: "key-own",
        total_spend: Some((5, 0)),
    },
    ExpectedRow {
        table: KeyTable::Active,
        id: "key-linked",
        total_spend: Some((5, 0)),
    },
    ExpectedRow {
        table: KeyTable::Active,
        id: "key-plain",
        total_spend: Some((5, 0)),
    },
    ExpectedRow {
        table: KeyTable::Archived,
        id: "archived-own",
        total_spend: Some((7, 0)),
    },
    ExpectedRow {
        table: KeyTable::Archived,
        id: "archived-plain",
        total_spend: Some((7, 0)),
    },
];

pub(super) const OWN_LINKED_BUDGETS: [BudgetPlan<'_>; 2] = [
    BudgetPlan {
        budget_id: "budget-linked",
        budget_duration: Some("30d"),
    },
    BudgetPlan {
        budget_id: "budget-plain",
        budget_duration: None,
    },
];

pub(super) const OWN_LINKED_KEYS: [KeyPlan<'_>; 3] = [
    KeyPlan {
        table: KeyTable::Active,
        id: "key-own",
        token: "key-own",
        spend: (0, 0),
        total_spend: (0, 0),
        budget_duration: Some("1d"),
        budget_id: None,
    },
    KeyPlan {
        table: KeyTable::Active,
        id: "key-linked",
        token: "key-linked",
        spend: (0, 0),
        total_spend: (0, 0),
        budget_duration: None,
        budget_id: Some("budget-linked"),
    },
    KeyPlan {
        table: KeyTable::Active,
        id: "key-no-reset",
        token: "key-no-reset",
        spend: (0, 0),
        total_spend: (0, 0),
        budget_duration: None,
        budget_id: Some("budget-plain"),
    },
];

pub(super) const OWN_LINKED_LOGS: [LogPlan<'_>; 3] = [
    LogPlan {
        request_id: "log-own",
        api_key: "key-own",
        spend: (3, 0),
        total_tokens: 0,
    },
    LogPlan {
        request_id: "log-linked",
        api_key: "key-linked",
        spend: (4, 0),
        total_tokens: 0,
    },
    LogPlan {
        request_id: "log-no-reset",
        api_key: "key-no-reset",
        spend: (9, 0),
        total_tokens: 0,
    },
];

pub(super) const OWN_LINKED_TOTALS: [ExpectedRow<'_>; 3] = [
    ExpectedRow {
        table: KeyTable::Active,
        id: "key-own",
        total_spend: Some((3, 0)),
    },
    ExpectedRow {
        table: KeyTable::Active,
        id: "key-linked",
        total_spend: Some((4, 0)),
    },
    ExpectedRow {
        table: KeyTable::Active,
        id: "key-no-reset",
        total_spend: Some((0, 0)),
    },
];

pub(super) const OWN_LINKED_SCENARIO: HistoryScenario<'static> = HistoryScenario {
    budgets: &OWN_LINKED_BUDGETS,
    keys: &OWN_LINKED_KEYS,
    logs: &OWN_LINKED_LOGS,
    outcome: BackfillOutcome::KeyTotalSpendFromLogs {
        active_keys: 2,
        archived_keys: 0,
    },
    totals: &OWN_LINKED_TOTALS,
};

pub(super) const EMPTY_DURATION_BUDGETS: [BudgetPlan<'_>; 2] = [
    BudgetPlan {
        budget_id: "budget-empty",
        budget_duration: Some(""),
    },
    BudgetPlan {
        budget_id: "budget-null",
        budget_duration: None,
    },
];

pub(super) const EMPTY_DURATION_KEYS: [KeyPlan<'_>; 3] = [
    KeyPlan {
        table: KeyTable::Active,
        id: "key-empty",
        token: "key-empty",
        spend: (0, 0),
        total_spend: (0, 0),
        budget_duration: Some(""),
        budget_id: None,
    },
    KeyPlan {
        table: KeyTable::Active,
        id: "key-empty-linked",
        token: "key-empty-linked",
        spend: (0, 0),
        total_spend: (0, 0),
        budget_duration: None,
        budget_id: Some("budget-empty"),
    },
    KeyPlan {
        table: KeyTable::Active,
        id: "key-null-linked",
        token: "key-null-linked",
        spend: (0, 0),
        total_spend: (0, 0),
        budget_duration: None,
        budget_id: Some("budget-null"),
    },
];

pub(super) const EMPTY_DURATION_LOGS: [LogPlan<'_>; 3] = [
    LogPlan {
        request_id: "log-empty",
        api_key: "key-empty",
        spend: (25, 1),
        total_tokens: 0,
    },
    LogPlan {
        request_id: "log-empty-linked",
        api_key: "key-empty-linked",
        spend: (15, 1),
        total_tokens: 0,
    },
    LogPlan {
        request_id: "log-null-linked",
        api_key: "key-null-linked",
        spend: (7, 0),
        total_tokens: 0,
    },
];

pub(super) const EMPTY_DURATION_TOTALS: [ExpectedRow<'_>; 3] = [
    ExpectedRow {
        table: KeyTable::Active,
        id: "key-empty",
        total_spend: Some((25, 1)),
    },
    ExpectedRow {
        table: KeyTable::Active,
        id: "key-empty-linked",
        total_spend: Some((15, 1)),
    },
    ExpectedRow {
        table: KeyTable::Active,
        id: "key-null-linked",
        total_spend: Some((0, 0)),
    },
];

pub(super) const EMPTY_DURATION_SCENARIO: HistoryScenario<'static> = HistoryScenario {
    budgets: &EMPTY_DURATION_BUDGETS,
    keys: &EMPTY_DURATION_KEYS,
    logs: &EMPTY_DURATION_LOGS,
    outcome: BackfillOutcome::KeyTotalSpendFromLogs {
        active_keys: 2,
        archived_keys: 0,
    },
    totals: &EMPTY_DURATION_TOTALS,
};

pub(super) const ARCHIVE_DISTINCT_KEYS: [KeyPlan<'_>; 3] = [
    KeyPlan {
        table: KeyTable::Archived,
        id: "life-one",
        token: "archived-token",
        spend: (1, 0),
        total_spend: (0, 0),
        budget_duration: Some("1d"),
        budget_id: None,
    },
    KeyPlan {
        table: KeyTable::Archived,
        id: "life-two",
        token: "archived-token",
        spend: (2, 0),
        total_spend: (0, 0),
        budget_duration: Some("1d"),
        budget_id: None,
    },
    KeyPlan {
        table: KeyTable::Archived,
        id: "life-plain",
        token: "archived-token",
        spend: (0, 0),
        total_spend: (0, 0),
        budget_duration: None,
        budget_id: None,
    },
];

pub(super) const ARCHIVE_DISTINCT_LOGS: [LogPlan<'_>; 1] = [LogPlan {
    request_id: "log-token",
    api_key: "archived-token",
    spend: (3, 0),
    total_tokens: 0,
}];

pub(super) const ARCHIVE_DISTINCT_TOTALS: [ExpectedRow<'_>; 3] = [
    ExpectedRow {
        table: KeyTable::Archived,
        id: "life-one",
        total_spend: Some((3, 0)),
    },
    ExpectedRow {
        table: KeyTable::Archived,
        id: "life-two",
        total_spend: Some((3, 0)),
    },
    ExpectedRow {
        table: KeyTable::Archived,
        id: "life-plain",
        total_spend: Some((0, 0)),
    },
];

pub(super) const ARCHIVE_DISTINCT_SCENARIO: HistoryScenario<'static> = HistoryScenario {
    budgets: &[],
    keys: &ARCHIVE_DISTINCT_KEYS,
    logs: &ARCHIVE_DISTINCT_LOGS,
    outcome: BackfillOutcome::KeyTotalSpendFromLogs {
        active_keys: 0,
        archived_keys: 2,
    },
    totals: &ARCHIVE_DISTINCT_TOTALS,
};

pub(super) const MIXED_ARCHIVE_BUDGETS: [BudgetPlan<'_>; 1] = [BudgetPlan {
    budget_id: "budget-reset",
    budget_duration: Some("1d"),
}];

pub(super) const MIXED_ARCHIVE_KEYS: [KeyPlan<'_>; 4] = [
    KeyPlan {
        table: KeyTable::Archived,
        id: "own-reset",
        token: "mixed-token",
        spend: (0, 0),
        total_spend: (0, 0),
        budget_duration: Some("1d"),
        budget_id: None,
    },
    KeyPlan {
        table: KeyTable::Archived,
        id: "own-plain",
        token: "mixed-token",
        spend: (0, 0),
        total_spend: (0, 0),
        budget_duration: None,
        budget_id: None,
    },
    KeyPlan {
        table: KeyTable::Archived,
        id: "linked-reset",
        token: "linked-token",
        spend: (0, 0),
        total_spend: (0, 0),
        budget_duration: None,
        budget_id: Some("budget-reset"),
    },
    KeyPlan {
        table: KeyTable::Archived,
        id: "linked-plain",
        token: "linked-token",
        spend: (0, 0),
        total_spend: (0, 0),
        budget_duration: None,
        budget_id: None,
    },
];

pub(super) const MIXED_ARCHIVE_LOGS: [LogPlan<'_>; 2] = [
    LogPlan {
        request_id: "log-mixed",
        api_key: "mixed-token",
        spend: (2, 0),
        total_tokens: 0,
    },
    LogPlan {
        request_id: "log-linked",
        api_key: "linked-token",
        spend: (3, 0),
        total_tokens: 0,
    },
];

pub(super) const MIXED_ARCHIVE_TOTALS: [ExpectedRow<'_>; 4] = [
    ExpectedRow {
        table: KeyTable::Archived,
        id: "own-reset",
        total_spend: Some((2, 0)),
    },
    ExpectedRow {
        table: KeyTable::Archived,
        id: "own-plain",
        total_spend: Some((0, 0)),
    },
    ExpectedRow {
        table: KeyTable::Archived,
        id: "linked-reset",
        total_spend: Some((3, 0)),
    },
    ExpectedRow {
        table: KeyTable::Archived,
        id: "linked-plain",
        total_spend: Some((0, 0)),
    },
];

pub(super) const MIXED_ARCHIVE_SCENARIO: HistoryScenario<'static> = HistoryScenario {
    budgets: &MIXED_ARCHIVE_BUDGETS,
    keys: &MIXED_ARCHIVE_KEYS,
    logs: &MIXED_ARCHIVE_LOGS,
    outcome: BackfillOutcome::KeyTotalSpendFromLogs {
        active_keys: 0,
        archived_keys: 2,
    },
    totals: &MIXED_ARCHIVE_TOTALS,
};

pub(super) const SHORT_HISTORY_KEYS: [KeyPlan<'_>; 4] = [
    KeyPlan {
        table: KeyTable::Active,
        id: "key-short",
        token: "key-short",
        spend: (5, 0),
        total_spend: (5, 0),
        budget_duration: Some("1d"),
        budget_id: None,
    },
    KeyPlan {
        table: KeyTable::Active,
        id: "key-missing",
        token: "key-missing",
        spend: (2, 0),
        total_spend: (2, 0),
        budget_duration: Some("1d"),
        budget_id: None,
    },
    KeyPlan {
        table: KeyTable::Active,
        id: "key-null-sum",
        token: "key-null-sum",
        spend: (1, 0),
        total_spend: (1, 0),
        budget_duration: Some("1d"),
        budget_id: None,
    },
    KeyPlan {
        table: KeyTable::Active,
        id: "key-plain",
        token: "key-plain",
        spend: (9, 0),
        total_spend: (0, 0),
        budget_duration: None,
        budget_id: None,
    },
];

pub(super) const SHORT_HISTORY_LOGS: [LogPlan<'_>; 2] = [
    LogPlan {
        request_id: "log-short",
        api_key: "key-short",
        spend: (2, 0),
        total_tokens: 0,
    },
    LogPlan {
        request_id: "log-plain",
        api_key: "key-plain",
        spend: (9, 0),
        total_tokens: 0,
    },
];

pub(super) const SHORT_HISTORY_TOTALS: [ExpectedRow<'_>; 4] = [
    ExpectedRow {
        table: KeyTable::Active,
        id: "key-short",
        total_spend: Some((5, 0)),
    },
    ExpectedRow {
        table: KeyTable::Active,
        id: "key-missing",
        total_spend: Some((2, 0)),
    },
    ExpectedRow {
        table: KeyTable::Active,
        id: "key-null-sum",
        total_spend: Some((1, 0)),
    },
    ExpectedRow {
        table: KeyTable::Active,
        id: "key-plain",
        total_spend: Some((0, 0)),
    },
];

pub(super) const FLOOR_FAILURE_KEYS: [KeyPlan<'_>; 2] = [
    KeyPlan {
        table: KeyTable::Active,
        id: "key-active",
        token: "key-active",
        spend: (5, 0),
        total_spend: (1, 0),
        budget_duration: None,
        budget_id: None,
    },
    KeyPlan {
        table: KeyTable::Archived,
        id: "life-one",
        token: "token-one",
        spend: (7, 0),
        total_spend: (2, 0),
        budget_duration: None,
        budget_id: None,
    },
];

pub(super) const HISTORY_FAILURE_KEYS: [KeyPlan<'_>; 2] = [
    KeyPlan {
        table: KeyTable::Active,
        id: "key-active",
        token: "key-active",
        spend: (1, 0),
        total_spend: (1, 0),
        budget_duration: Some("1d"),
        budget_id: None,
    },
    KeyPlan {
        table: KeyTable::Archived,
        id: "life-one",
        token: "token-one",
        spend: (2, 0),
        total_spend: (2, 0),
        budget_duration: Some("1d"),
        budget_id: None,
    },
];

pub(super) const HISTORY_FAILURE_LOGS: [LogPlan<'_>; 2] = [
    LogPlan {
        request_id: "log-active",
        api_key: "key-active",
        spend: (4, 0),
        total_tokens: 0,
    },
    LogPlan {
        request_id: "log-archived",
        api_key: "token-one",
        spend: (6, 0),
        total_tokens: 0,
    },
];

pub(super) const FLOOR_FAILURE_SCENARIO: FailureScenario<'static> = FailureScenario {
    operation: Operation::Floor,
    keys: &FLOOR_FAILURE_KEYS,
    logs: &[],
};

pub(super) const HISTORY_FAILURE_SCENARIO: FailureScenario<'static> = FailureScenario {
    operation: Operation::History,
    keys: &HISTORY_FAILURE_KEYS,
    logs: &HISTORY_FAILURE_LOGS,
};

pub(super) const NULL_LIFETIME_TOTALS: [ExpectedRow<'_>; 8] = [
    ExpectedRow {
        table: KeyTable::Active,
        id: "null-spend",
        total_spend: Some((1, 0)),
    },
    ExpectedRow {
        table: KeyTable::Active,
        id: "null-total",
        total_spend: None,
    },
    ExpectedRow {
        table: KeyTable::Active,
        id: "both-null",
        total_spend: None,
    },
    ExpectedRow {
        table: KeyTable::Active,
        id: "below",
        total_spend: Some((5, 0)),
    },
    ExpectedRow {
        table: KeyTable::Active,
        id: "equal",
        total_spend: Some((5, 0)),
    },
    ExpectedRow {
        table: KeyTable::Active,
        id: "higher",
        total_spend: Some((10, 0)),
    },
    ExpectedRow {
        table: KeyTable::Archived,
        id: "archived-below",
        total_spend: Some((5, 0)),
    },
    ExpectedRow {
        table: KeyTable::Archived,
        id: "archived-null",
        total_spend: Some((1, 0)),
    },
];
