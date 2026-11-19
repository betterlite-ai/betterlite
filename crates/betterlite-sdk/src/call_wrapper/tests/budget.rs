use rust_decimal::Decimal;

use super::support::Stub;
use super::support::chat_request;
use super::support::fixture;
use super::support::response;
use super::support::run_sync;
use super::support::value;
use crate::call_wrapper::budget::BudgetState;
use crate::call_wrapper::error::class_name;
use crate::call_wrapper::state::WrapperOptions;

fn budget(current_cost: Decimal, max_budget: Option<Decimal>) -> BudgetState {
    BudgetState {
        max_budget,
        current_cost,
    }
}

const MESSAGE: &str = "Budget has been exceeded! Current cost: 2.5, Max budget: 1.5";

#[test]
fn sync_budget_exceeded_stops_before_the_stub() {
    let mut fixture = fixture();
    fixture.wrapper.budget = budget(Decimal::new(25, 1), Some(Decimal::new(15, 1)));
    let stub = Stub::new("completion");
    let error =
        run_sync(&mut fixture, &stub, chat_request("{}")).expect_err("the budget stops the call");
    assert_eq!(error.to_string(), MESSAGE);
    assert_eq!(class_name(&error.class), "BudgetExceededError");
    assert_eq!(stub.call_count(), 0);
}

#[tokio::test]
async fn async_budget_exceeded_stops_before_the_stub() {
    let mut fixture = fixture();
    fixture.wrapper.budget = budget(Decimal::new(25, 1), Some(Decimal::new(15, 1)));
    let stub = Stub::new("acompletion");
    let error = fixture
        .wrapper
        .call(&stub, chat_request("{}"), WrapperOptions::default())
        .await
        .expect_err("the budget stops the call");
    assert_eq!(error.to_string(), MESSAGE);
    assert_eq!(class_name(&error.class), "BudgetExceededError");
    assert_eq!(stub.call_count(), 0);
}

#[test]
fn a_cost_at_the_budget_runs() {
    let mut fixture = fixture();
    fixture.wrapper.budget = budget(Decimal::new(15, 1), Some(Decimal::new(15, 1)));
    let stub = Stub::responding("completion", response(value(r#"{"id":"chat-1"}"#)));
    run_sync(&mut fixture, &stub, chat_request("{}")).expect("the boundary does not stop the call");
    assert_eq!(stub.call_count(), 1);
}

#[test]
fn a_cost_just_past_the_budget_stops() {
    let mut fixture = fixture();
    fixture.wrapper.budget = budget(Decimal::new(151, 2), Some(Decimal::new(15, 1)));
    let stub = Stub::new("completion");
    run_sync(&mut fixture, &stub, chat_request("{}")).expect_err("a hundredth stops the call");
    assert_eq!(stub.call_count(), 0);
}

#[test]
fn a_cost_without_a_budget_runs() {
    let mut fixture = fixture();
    fixture.wrapper.budget = budget(Decimal::new(1000, 1), None);
    let stub = Stub::new("completion");
    run_sync(&mut fixture, &stub, chat_request("{}")).expect("no budget means no check");
    assert_eq!(stub.call_count(), 1);
}
