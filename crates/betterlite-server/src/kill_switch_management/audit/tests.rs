#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use betterlite_core::agents::{AgentKillSwitchMethod, AgentKillSwitchResult};
use chrono::DateTime;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{KillSwitchAuditContext, build_kill_switch_audit_log};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::identity::CallerIdentity;
use crate::audit_events::record::AuditLogRecord;
use crate::kill_switch_management::support::FixedClock;

const TIMESTAMP: &str = "2024-05-01T00:00:00+00:00";
const AUDIT_ID: &str = "audit-1";

fn fixed_clock() -> FixedClock {
    FixedClock {
        now: DateTime::parse_from_rfc3339(TIMESTAMP).expect("a timestamp"),
    }
}

fn caller(user_id: Option<&str>, api_key: Option<&str>) -> CallerIdentity {
    CallerIdentity {
        user_id: user_id.map(String::from),
        api_key: api_key.map(SecretString::from),
        metadata: None,
        team_metadata: None,
    }
}

fn succeeded_result() -> AgentKillSwitchResult {
    AgentKillSwitchResult {
        agent_id: String::from("agent-1"),
        url: String::from("https://agent.example/stop"),
        method: AgentKillSwitchMethod::Post,
        status_code: Some(200),
        response_body: Some(String::from("stopped")),
        error: None,
    }
}

fn audit_log(
    result: &AgentKillSwitchResult,
    caller: &CallerIdentity,
    admin: Option<&str>,
) -> AuditLogRecord {
    build_kill_switch_audit_log(
        result,
        KillSwitchAuditContext {
            caller,
            litellm_proxy_admin_name: admin,
            clock: &fixed_clock(),
            id: String::from(AUDIT_ID),
        },
    )
    .expect("the audit log is built")
}

#[test]
fn the_audit_log_records_the_fired_action_on_the_agent_table() {
    let record = audit_log(&succeeded_result(), &caller(Some("operator"), None), None);
    assert_eq!(record.action, AuditAction::KillSwitchFired);
    assert_eq!(record.action.as_str(), "kill_switch_fired");
    assert_eq!(record.table_name, AuditTable::Agent);
    assert_eq!(record.table_name.as_str(), "LiteLLM_AgentsTable");
    assert_eq!(record.object_id, "agent-1");
    assert!(record.before_value.is_none());
}

#[test]
fn the_audit_log_uses_the_injected_id_and_timestamp() {
    let record = audit_log(&succeeded_result(), &caller(None, None), None);
    assert_eq!(record.id, AUDIT_ID);
    assert_eq!(
        record.updated_at,
        DateTime::parse_from_rfc3339(TIMESTAMP).expect("a timestamp")
    );
}

#[test]
fn the_audit_log_stores_the_result_as_json_text() {
    let record = audit_log(&succeeded_result(), &caller(None, None), None);
    assert_eq!(
        record.updated_values,
        Some(JsonValue::String(String::from(
            r#"{"agent_id":"agent-1","url":"https://agent.example/stop","method":"POST","status_code":200,"response_body":"stopped"}"#
        )))
    );
}

#[test]
fn the_audit_log_leaves_out_the_result_fields_that_are_absent() {
    let mut failed = succeeded_result();
    failed.status_code = None;
    failed.response_body = None;
    failed.error = Some(String::from("ConnectError"));
    let record = audit_log(&failed, &caller(None, None), None);
    assert_eq!(
        record.updated_values,
        Some(JsonValue::String(String::from(
            r#"{"agent_id":"agent-1","url":"https://agent.example/stop","method":"POST","error":"ConnectError"}"#
        )))
    );
}

#[rstest]
#[case::operator_over_admin(Some("operator"), Some("admin"), Some("operator"))]
#[case::admin_when_no_user(None, Some("admin"), Some("admin"))]
#[case::nothing_without_user_and_admin(None, None, None)]
#[case::admin_for_an_empty_user(Some(""), Some("admin"), Some("admin"))]
fn the_audit_log_names_the_caller_then_the_admin(
    #[case] user_id: Option<&str>,
    #[case] admin: Option<&str>,
    #[case] changed_by: Option<&str>,
) {
    let record = audit_log(&succeeded_result(), &caller(user_id, None), admin);
    assert_eq!(record.changed_by.as_deref(), changed_by);
}

#[test]
fn the_audit_log_carries_the_callers_api_key() {
    let record = audit_log(
        &succeeded_result(),
        &caller(Some("operator"), Some("sk-caller")),
        None,
    );
    assert_eq!(
        record
            .changed_by_api_key
            .as_ref()
            .map(|key| String::from(key.expose_secret())),
        Some(String::from("sk-caller"))
    );
}

#[test]
fn the_audit_log_has_no_api_key_when_the_caller_has_none() {
    let record = audit_log(&succeeded_result(), &caller(Some("operator"), None), None);
    assert!(record.changed_by_api_key.is_none());
}
