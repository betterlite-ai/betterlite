#![cfg(test)]

use std::future::Future as _;
use std::sync::{Mutex, PoisonError};
use std::task::{Context, Poll, Waker};

use tokio::task::yield_now;

use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;
use crate::agent_registry::tests::support::agent;

#[test]
fn registry_methods_work_without_the_reconcile_lock() {
    let registry = AgentRegistry::default();
    registry.register_agent(agent("id", "name"));
    assert_eq!(registry.get_agent_list(None).len(), 1);
    registry.deregister_agent("name");
    assert!(registry.get_agent_list(None).is_empty());
}

#[tokio::test]
async fn a_second_guard_waits_for_the_first() {
    let registry = AgentRegistry::default();
    let mut first = Box::pin(registry.reconcile());
    let mut context = Context::from_waker(Waker::noop());
    let Poll::Ready(guard) = first.as_mut().poll(&mut context) else {
        panic!("the first guard is available");
    };
    let mut second = Box::pin(registry.reconcile());
    assert!(matches!(second.as_mut().poll(&mut context), Poll::Pending));
    drop(guard);
    assert!(matches!(second.as_mut().poll(&mut context), Poll::Ready(_)));
}

async fn first_sequence(registry: &AgentRegistry, events: &Mutex<Vec<&'static str>>) {
    let guard = registry.reconcile().await;
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push("first-read");
    yield_now().await;
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push("first-register");
    registry.register_agent(agent("a1", "alpha"));
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push("first-release");
    drop(guard);
}

async fn second_sequence(registry: &AgentRegistry, events: &Mutex<Vec<&'static str>>) {
    let guard = registry.reconcile().await;
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push("second-read");
    registry.register_agent(agent("b1", "béta"));
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push("second-release");
    drop(guard);
}

#[tokio::test]
async fn two_read_then_register_sequences_do_not_interleave() {
    let registry = AgentRegistry::default();
    let events = Mutex::new(Vec::new());
    tokio::join!(
        first_sequence(&registry, &events),
        second_sequence(&registry, &events)
    );
    let order = events.into_inner().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        order,
        vec![
            "first-read",
            "first-register",
            "first-release",
            "second-read",
            "second-release",
        ]
    );
}
