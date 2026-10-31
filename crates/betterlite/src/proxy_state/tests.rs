#![cfg(test)]

use super::{ProxyState, ProxyStateVariable};

#[test]
fn starts_at_zero() {
    let state = ProxyState::default();
    assert_eq!(
        state.get_proxy_state_variable(ProxyStateVariable::SpendLogsRowCount),
        0
    );
}

#[test]
fn set_then_get_round_trips() {
    let state = ProxyState::default();
    state.set_proxy_state_variable(ProxyStateVariable::SpendLogsRowCount, 42);
    assert_eq!(
        state.get_proxy_state_variable(ProxyStateVariable::SpendLogsRowCount),
        42
    );
}
