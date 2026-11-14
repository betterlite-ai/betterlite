use std::sync::Arc;

use crate::langfuse_logger::LangfuseLoggerError;
use crate::langfuse_logger::budget::{
    CLIENT_LIMIT_VARIABLE, DEFAULT_CLIENT_LIMIT, LangfuseBudgetError, LangfuseClientBudget,
};
use crate::langfuse_logger::langfuse::LangFuseLogger;

use super::support;

#[test]
fn the_limit_comes_from_the_environment() {
    let unset =
        LangfuseClientBudget::from_environment(&support::lookup(&[])).expect("the default limit");
    assert_eq!(unset.limit, DEFAULT_CLIENT_LIMIT);
    let empty =
        LangfuseClientBudget::from_environment(&support::lookup(&[(CLIENT_LIMIT_VARIABLE, "")]))
            .expect("the default limit");
    assert_eq!(empty.limit, DEFAULT_CLIENT_LIMIT);
    let configured =
        LangfuseClientBudget::from_environment(&support::lookup(&[(CLIENT_LIMIT_VARIABLE, "3")]))
            .expect("the configured limit");
    assert_eq!(configured.limit, 3);
    let refused = LangfuseClientBudget::from_environment(&support::lookup(&[(
        CLIENT_LIMIT_VARIABLE,
        "many",
    )]))
    .err()
    .expect("a non-number is refused");
    assert!(matches!(refused, LangfuseBudgetError::Limit(_)));
}

#[tokio::test]
async fn a_full_budget_refuses_another_logger() {
    let services = support::services(&[("LANGFUSE_MOCK", "true")], 1);
    let _first = LangFuseLogger::new(&services, &support::request("pk-budget-1", "sk-budget-1"))
        .await
        .expect("the first logger builds");
    let error = LangFuseLogger::new(&services, &support::request("pk-budget-2", "sk-budget-2"))
        .await
        .expect_err("the budget refuses the second");
    assert!(matches!(error, LangfuseLoggerError::Budget(_)));
    assert_eq!(
        error.to_string(),
        "Max langfuse clients reached: 1 is greater than 1"
    );
}

#[tokio::test]
async fn a_failed_client_build_does_not_hold_a_slot() {
    let budget = LangfuseClientBudget::new(1);
    let failing = support::services_with(
        &[],
        budget.clone(),
        Arc::new(support::FailingBuilder),
        Arc::new(support::FixedIds),
    );
    let error = LangFuseLogger::new(&failing, &support::request("pk-slot", "sk-slot"))
        .await
        .expect_err("the client build fails");
    assert!(matches!(error, LangfuseLoggerError::Client(_)));
    let mock = support::services_with(
        &[("LANGFUSE_MOCK", "true")],
        budget,
        support::real_builder(),
        Arc::new(support::FixedIds),
    );
    LangFuseLogger::new(&mock, &support::request("pk-slot-2", "sk-slot-2"))
        .await
        .expect("the slot was given back");
}

#[tokio::test]
async fn stop_returns_the_slot() {
    let services = support::services(&[("LANGFUSE_MOCK", "true")], 1);
    let logger = LangFuseLogger::new(&services, &support::request("pk-stop", "sk-stop"))
        .await
        .expect("the logger builds");
    logger.stop();
    LangFuseLogger::new(&services, &support::request("pk-stop-2", "sk-stop-2"))
        .await
        .expect("the slot was given back");
}

#[tokio::test]
async fn a_logger_dropped_without_stop_frees_its_slot() {
    let services = support::services(&[("LANGFUSE_MOCK", "true")], 1);
    let logger = LangFuseLogger::new(&services, &support::request("pk-drop", "sk-drop"))
        .await
        .expect("the logger builds");
    drop(logger);
    LangFuseLogger::new(&services, &support::request("pk-drop-2", "sk-drop-2"))
        .await
        .expect("the dropped logger's slot is free");
}

#[tokio::test]
async fn a_stopped_and_dropped_logger_frees_one_slot() {
    let services = support::services(&[("LANGFUSE_MOCK", "true")], 2);
    let first = LangFuseLogger::new(&services, &support::request("pk-both-1", "sk-both-1"))
        .await
        .expect("the first logger builds");
    let _second = LangFuseLogger::new(&services, &support::request("pk-both-2", "sk-both-2"))
        .await
        .expect("the second logger builds");
    first.stop();
    drop(first);
    let _third = LangFuseLogger::new(&services, &support::request("pk-both-3", "sk-both-3"))
        .await
        .expect("the one freed slot is reused");
    let error = LangFuseLogger::new(&services, &support::request("pk-both-4", "sk-both-4"))
        .await
        .expect_err("only one slot was freed");
    assert!(matches!(error, LangfuseLoggerError::Budget(_)));
}
