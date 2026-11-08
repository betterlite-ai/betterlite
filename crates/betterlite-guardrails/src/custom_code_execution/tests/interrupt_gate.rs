use std::sync::atomic::Ordering;
use std::thread;

use super::super::gate::InterruptGate;

#[test]
fn a_closed_gate_refuses_to_interrupt() {
    let gate = InterruptGate::new();
    assert!(!gate.interrupt());
    assert!(!gate.interrupted.load(Ordering::SeqCst));
}

#[test]
fn an_open_gate_accepts_an_interrupt() {
    let gate = InterruptGate::new();
    gate.open();
    assert!(gate.interrupt());
    assert!(gate.interrupted.load(Ordering::SeqCst));
}

#[test]
fn closing_the_gate_prevents_a_stale_thread_hit() {
    let gate = InterruptGate::new();
    gate.open();
    assert!(gate.interrupt());
    gate.close();
    assert!(!gate.interrupt());
    assert!(!gate.interrupted.load(Ordering::SeqCst));
}

#[test]
fn reopening_clears_the_previous_interrupt() {
    let gate = InterruptGate::new();
    gate.open();
    assert!(gate.interrupt());
    gate.close();
    gate.open();
    assert!(!gate.interrupted.load(Ordering::SeqCst));
}

#[test]
fn clones_share_the_same_gate() {
    let gate = InterruptGate::new();
    let clone = gate.clone();
    gate.open();
    assert!(clone.interrupt());
    assert!(gate.interrupted.load(Ordering::SeqCst));
}

#[test]
fn concurrent_interrupts_all_see_the_open_gate() {
    let gate = InterruptGate::new();
    gate.open();
    thread::scope(|scope| {
        let first = gate.clone();
        scope.spawn(move || assert!(first.interrupt()));
        let second = gate.clone();
        scope.spawn(move || assert!(second.interrupt()));
        let third = gate.clone();
        scope.spawn(move || assert!(third.interrupt()));
        let fourth = gate.clone();
        scope.spawn(move || assert!(fourth.interrupt()));
        let fifth = gate.clone();
        scope.spawn(move || assert!(fifth.interrupt()));
        let sixth = gate.clone();
        scope.spawn(move || assert!(sixth.interrupt()));
        let seventh = gate.clone();
        scope.spawn(move || assert!(seventh.interrupt()));
        let eighth = gate.clone();
        scope.spawn(move || assert!(eighth.interrupt()));
    });
    assert!(gate.interrupted.load(Ordering::SeqCst));
}
