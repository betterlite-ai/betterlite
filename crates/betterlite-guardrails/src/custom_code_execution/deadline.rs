use std::cell::Cell;
use std::future::Future;
use std::mem::replace;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Instant;

use super::execution::error::ExecutionInterrupted;
use super::gate::InterruptGate;

thread_local! {
    static DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
    static GATE: Mutex<Option<Arc<InterruptGate>>> = const { Mutex::new(None) };
}

// A poisoned deadline lock still holds the gate the next check needs, so the poison is ignored
fn gate_interrupted() -> bool {
    GATE.with(|slot| {
        slot.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|gate| gate.interrupted.load(Ordering::SeqCst))
    })
}

fn replace_gate(gate: Option<Arc<InterruptGate>>) -> Option<Arc<InterruptGate>> {
    GATE.with(|slot| {
        replace(
            &mut *slot.lock().unwrap_or_else(PoisonError::into_inner),
            gate,
        )
    })
}

// Restores the deadline the calling scope had when this guard drops, leaving no budget for the next call.
pub struct BudgetGuard {
    deadline: Option<Instant>,
    gate: Option<Arc<InterruptGate>>,
}

impl Drop for BudgetGuard {
    fn drop(&mut self) {
        DEADLINE.with(|slot| slot.set(self.deadline));
        replace_gate(self.gate.take());
    }
}

#[must_use]
pub fn enter(deadline: Instant, gate: Arc<InterruptGate>) -> BudgetGuard {
    let previous = BudgetGuard {
        deadline: DEADLINE.with(Cell::get),
        gate: replace_gate(Some(gate)),
    };
    DEADLINE.with(|slot| slot.set(Some(deadline)));
    previous
}

#[must_use]
pub fn past_deadline() -> bool {
    let now = Instant::now();
    let thread_deadline = DEADLINE
        .with(Cell::get)
        .is_some_and(|deadline| now > deadline);
    thread_deadline || gate_interrupted()
}

pub fn budget_ok() -> Result<(), ExecutionInterrupted> {
    if past_deadline() {
        return Err(ExecutionInterrupted);
    }
    Ok(())
}

// Publishes the deadline around each poll of the awaited future, and clears it before the runtime runs another task.
pub struct DeadlineScope<F> {
    deadline: Instant,
    pending: Pin<Box<F>>,
}

impl<F> Future for DeadlineScope<F>
where
    F: Future,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let previous = DEADLINE.with(Cell::get);
        DEADLINE.with(|slot| slot.set(Some(this.deadline)));
        let polled = this.pending.as_mut().poll(cx);
        DEADLINE.with(|slot| slot.set(previous));
        polled
    }
}

pub fn with_deadline<F>(deadline: Instant, pending: F) -> DeadlineScope<F>
where
    F: Future,
{
    DeadlineScope {
        deadline,
        pending: Box::pin(pending),
    }
}

pub struct Budgeted<I> {
    items: I,
}

impl<T, I> Iterator for Budgeted<I>
where
    I: Iterator<Item = T>,
{
    type Item = Result<T, ExecutionInterrupted>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.items.next()?;
        match budget_ok() {
            Ok(()) => Some(Ok(item)),
            Err(interrupted) => Some(Err(interrupted)),
        }
    }
}

// `_getiter_`: every for loop and comprehension takes its items through this, so a spent budget raises instead of yielding.
pub fn budgeted_iter<T, I>(items: I) -> Budgeted<I::IntoIter>
where
    I: IntoIterator<Item = T>,
{
    Budgeted {
        items: items.into_iter(),
    }
}
