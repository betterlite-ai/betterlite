#![cfg(test)]

use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use rust_decimal::Decimal;

use super::super::check::{BUDGET_EXCEEDED_STATUS, EndUserBudgetError, check_end_user_budget};
use super::support::{
    JournalBatch, ScriptedSpendRows, SpendCache, budget_fixture, end_user_row, money, reseed_over,
};
use crate::current_spend::CurrentSpend;
use crate::spend_counter_batch::batch::CounterValue;
use crate::spend_reseed::reseed::SpendCounterReseed;

const CHAT_ROUTE: &str = "/v1/chat/completions";

struct Rig {
    cache: SpendCache,
    batch: JournalBatch,
    reseed: SpendCounterReseed<ScriptedSpendRows>,
}

impl Rig {
    fn service(&self) -> CurrentSpend<'_, SpendCache, JournalBatch, ScriptedSpendRows> {
        CurrentSpend {
            cache: &self.cache,
            batch: &self.batch,
            reseed: &self.reseed,
            fail_closed_budget_enforcement: false,
        }
    }
}

fn rig(answer: Option<Decimal>) -> Rig {
    Rig {
        cache: SpendCache::new(),
        batch: JournalBatch::answering(answer.map(CounterValue::Amount)),
        reseed: reseed_over::<ScriptedSpendRows>(None),
    }
}

fn limited(max_budget: &str, spend: &str) -> LiteLLMEndUserTable {
    let relation = budget_fixture("limited", Some(max_budget));
    end_user_row("alice", spend, Some("limited"), Some(&relation))
}

#[tokio::test]
async fn info_route_skips_budget() {
    let rig = rig(Some(money("1000")));
    let end_user = limited("100", "0");
    check_end_user_budget(&end_user, "/key/info", &rig.service())
        .await
        .expect("an info route skips the spend read");
    assert!(rig.batch.reads().is_empty());
}

#[tokio::test]
async fn absent_budget_skips_spend() {
    let rig = rig(Some(money("1000")));
    let without_relation = end_user_row("alice", "0", None, None);
    check_end_user_budget(&without_relation, CHAT_ROUTE, &rig.service())
        .await
        .expect("a row with no relation has no ceiling");
    let without_max = budget_fixture("open", None);
    let open = end_user_row("alice", "0", Some("open"), Some(&without_max));
    check_end_user_budget(&open, CHAT_ROUTE, &rig.service())
        .await
        .expect("a budget with no max_budget has no ceiling");
    assert!(rig.batch.reads().is_empty());
}

#[tokio::test]
async fn authoritative_end_user_counter() {
    let rig = rig(Some(money("10")));
    let rows = ScriptedSpendRows::new(Some(money("200")));
    let reseed = reseed_over(Some(rows));
    let spend = CurrentSpend {
        cache: &rig.cache,
        batch: &rig.batch,
        reseed: &reseed,
        fail_closed_budget_enforcement: false,
    };
    // the cached counter is stale-low, so the end-user row it guards is the authoritative spend
    let end_user = limited("100", "200");
    let error = check_end_user_budget(&end_user, CHAT_ROUTE, &spend)
        .await
        .expect_err("the authoritative row spend decides over the stale counter");
    let EndUserBudgetError::Exceeded(exceeded) = error else {
        panic!("an overrun is a budget rejection");
    };
    assert_eq!(exceeded.details.current_cost, money("200"));
    assert_eq!(rig.batch.reads(), vec!["spend:end_user:alice".to_owned()]);
    assert_eq!(
        reseed.rows.as_ref().expect("the rows").lookups(),
        vec![r#"EndUser("alice")"#.to_owned()]
    );
}

#[tokio::test]
async fn equal_budget_allowed() {
    let rig = rig(Some(money("100")));
    let end_user = limited("100", "0");
    check_end_user_budget(&end_user, CHAT_ROUTE, &rig.service())
        .await
        .expect("spending exactly the budget is allowed");
}

#[tokio::test]
async fn zero_budget_rejects_positive() {
    let rig = rig(Some(money("1")));
    let end_user = limited("0", "0");
    let error = check_end_user_budget(&end_user, CHAT_ROUTE, &rig.service())
        .await
        .expect_err("any positive spend exceeds a zero budget");
    let EndUserBudgetError::Exceeded(exceeded) = error else {
        panic!("a zero-budget overrun is a budget rejection");
    };
    assert_eq!(exceeded.details.current_cost, money("1"));
    assert_eq!(exceeded.details.max_budget, money("0"));
}

#[tokio::test]
async fn over_budget_fields() {
    let rig = rig(Some(money("150")));
    let end_user = limited("100", "0");
    let error = check_end_user_budget(&end_user, CHAT_ROUTE, &rig.service())
        .await
        .expect_err("spending past the budget is rejected");
    let EndUserBudgetError::Exceeded(exceeded) = error else {
        panic!("an overrun is a budget rejection");
    };
    let details = exceeded.details;
    assert_eq!(details.current_cost, money("150"));
    assert_eq!(details.max_budget, money("100"));
    assert_eq!(
        details.message.as_deref(),
        Some("ExceededBudget: End User=alice over budget. Spend=150, Budget=100")
    );
    assert_eq!(details.provider, "");
    assert_eq!(details.entity_type.as_deref(), Some("end_user"));
    assert_eq!(details.entity_id.as_deref(), Some("alice"));
    assert_eq!(details.status_code, BUDGET_EXCEEDED_STATUS);
    assert_eq!(details.status_code, 422);
}

#[tokio::test]
async fn spend_failure_propagates() {
    let rig = Rig {
        cache: SpendCache::new(),
        batch: JournalBatch::failing(),
        reseed: reseed_over::<ScriptedSpendRows>(None),
    };
    let end_user = limited("100", "0");
    let error = check_end_user_budget(&end_user, CHAT_ROUTE, &rig.service())
        .await
        .expect_err("a failed spend read is not a budget decision");
    assert!(matches!(error, EndUserBudgetError::Spend(_)));
}
