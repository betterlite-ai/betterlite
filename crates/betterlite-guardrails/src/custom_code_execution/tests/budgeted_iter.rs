use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::deadline::{budgeted_iter, enter};
use super::super::execution::error::ExecutionInterrupted;
use super::super::gate::InterruptGate;

#[test]
fn yields_every_item_when_no_budget_is_spent() {
    let items: Vec<Result<u32, ExecutionInterrupted>> = budgeted_iter(0..3).collect();
    assert_eq!(items.len(), 3);
    assert!(items.iter().all(Result::is_ok));
}

#[test]
fn raises_mid_iteration_once_the_deadline_passes() {
    let gate = InterruptGate::new();
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(2))
        .unwrap();
    let _budget = enter(deadline, Arc::new(gate));
    let mut items = budgeted_iter(0..100_000_000_u64);
    let seen = items.by_ref().take_while(Result::is_ok).count();
    assert!(
        items.next().is_some_and(|item| item.is_err()),
        "the deadline never stopped the loop after {seen} items"
    );
    assert!(seen < 100_000_000, "the loop yielded every item: {seen}");
}

#[test]
fn an_expired_deadline_stops_the_first_item() {
    let gate = InterruptGate::new();
    let deadline = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .unwrap();
    let _budget = enter(deadline, Arc::new(gate));
    let mut items = budgeted_iter(0_u8..3_u8);
    assert!(matches!(items.next(), Some(Err(ExecutionInterrupted))));
}

#[test]
fn an_interrupt_requested_through_the_gate_stops_the_next_item() {
    let gate = InterruptGate::new();
    gate.open();
    let deadline = Instant::now().checked_add(Duration::from_secs(30)).unwrap();
    let _budget = enter(deadline, Arc::new(gate.clone()));
    let mut items = budgeted_iter(0_u8..3_u8);
    assert!(matches!(items.next(), Some(Ok(0_u8))));
    assert!(gate.interrupt());
    assert!(matches!(items.next(), Some(Err(ExecutionInterrupted))));
    gate.close();
}
