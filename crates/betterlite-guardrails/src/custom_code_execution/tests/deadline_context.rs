use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::deadline::{budget_ok, enter, past_deadline, with_deadline};
use super::super::gate::InterruptGate;

fn past() -> Instant {
    Instant::now()
        .checked_sub(Duration::from_millis(5))
        .unwrap()
}

fn future() -> Instant {
    Instant::now().checked_add(Duration::from_secs(30)).unwrap()
}

#[test]
fn no_deadline_means_the_budget_is_not_spent() {
    assert!(!past_deadline());
    budget_ok().unwrap();
}

#[test]
fn a_deadline_in_the_future_is_not_spent() {
    let _budget = enter(future(), Arc::new(InterruptGate::new()));
    assert!(!past_deadline());
    budget_ok().unwrap();
}

#[test]
fn a_deadline_in_the_past_is_spent() {
    let _budget = enter(past(), Arc::new(InterruptGate::new()));
    assert!(past_deadline());
    assert!(budget_ok().is_err());
}

#[test]
fn leaving_a_scope_restores_the_previous_deadline() {
    let gate = InterruptGate::new();
    let outer = enter(future(), Arc::new(gate.clone()));
    assert!(!past_deadline());
    let inner = enter(past(), Arc::new(gate));
    assert!(past_deadline());
    drop(inner);
    assert!(!past_deadline());
    drop(outer);
    assert!(!past_deadline());
}

#[test]
fn an_interrupt_from_the_gate_spends_the_budget_early() {
    let gate = InterruptGate::new();
    gate.open();
    let _budget = enter(future(), Arc::new(gate.clone()));
    assert!(!past_deadline());
    assert!(gate.interrupt());
    assert!(past_deadline());
    gate.close();
}

#[tokio::test]
async fn the_task_deadline_is_visible_inside_the_awaited_task() {
    let spent = with_deadline(past(), async { past_deadline() }).await;
    assert!(spent);
    assert!(!past_deadline());
}

#[tokio::test]
async fn a_task_deadline_in_the_future_leaves_the_budget_alone() {
    let spent = with_deadline(future(), async { budget_ok().is_err() }).await;
    assert!(!spent);
}
