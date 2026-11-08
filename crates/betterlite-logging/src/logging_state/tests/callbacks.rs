use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{build, init, params, text};
use crate::logging_state::LoggingInit;
use crate::logging_state::callbacks::{CallbackKind, should_run_callback};
use crate::logging_state::types::CallbackPolicy;

enum Scenario {
    NoLog(CallbackKind),
    GlobalDisable,
    Plain,
    NonBoolean,
}

fn outcome(scenario: &Scenario) -> String {
    let no_log = params(&[("no-log", Json::Bool(true))]);
    match scenario {
        Scenario::NoLog(callback) => {
            let state = build(init());
            should_run_callback(&state, *callback, &no_log).to_string()
        }
        Scenario::GlobalDisable => {
            let state = build(LoggingInit {
                callback_policy: CallbackPolicy {
                    global_disable_no_log_param: true,
                },
                ..init()
            });
            should_run_callback(&state, CallbackKind::External, &no_log).to_string()
        }
        Scenario::Plain => {
            let state = build(init());
            should_run_callback(&state, CallbackKind::External, &params(&[])).to_string()
        }
        Scenario::NonBoolean => {
            let state = build(init());
            let quoted = params(&[("no-log", text("true"))]);
            should_run_callback(&state, CallbackKind::External, &quoted).to_string()
        }
    }
}

#[rstest]
#[case::no_log_skips_an_ordinary_callback(Scenario::NoLog(CallbackKind::External), "false")]
#[case::no_log_keeps_proxy_bookkeeping(Scenario::NoLog(CallbackKind::ProxyInternal), "true")]
#[case::global_disable_overrides_no_log(Scenario::GlobalDisable, "true")]
#[case::a_plain_request_runs_its_callbacks(Scenario::Plain, "true")]
#[case::only_json_true_disables(Scenario::NonBoolean, "true")]
fn logging_state_behavior_tests_behavior(#[case] scenario: Scenario, #[case] expected: &str) {
    assert_eq!(outcome(&scenario), expected);
}
