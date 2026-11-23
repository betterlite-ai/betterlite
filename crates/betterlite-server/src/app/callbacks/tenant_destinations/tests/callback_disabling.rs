use betterlite_auth::auth_admission::ports::TenantDestinations as _;
use betterlite_logging::otel::proxy_runtime::slot::request_slot_scope;

use super::support::{
    RecordingSinks, TENANT_A, allowed_hosts, attach, build_adapter, build_key_with_disable_list,
    build_key_with_team, build_runtime, callback_names, disable_header, langfuse_entry, logging,
    newrelic_entry, no_headers, setup, slot,
};

async fn key_list_disables() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let _fan_out = attach(&runtime, &sinks);
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let entries = [
        langfuse_entry(TENANT_A, "pk-a", "sk-a"),
        newrelic_entry("nr-a"),
    ]
    .join(",");
    let key = build_key_with_disable_list(r#""langfuse_otel""#, &entries);
    let (_, names) = request_slot_scope(async {
        adapter.seed(&key, &no_headers()).expect("the key seeds");
        callback_names(&slot(&runtime))
    })
    .await;
    assert_eq!(names, vec!["newrelic".to_owned()]);
}

async fn header_replaces_key_list() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let _fan_out = attach(&runtime, &sinks);
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let entries = [
        langfuse_entry(TENANT_A, "pk-a", "sk-a"),
        newrelic_entry("nr-a"),
    ]
    .join(",");
    let key = build_key_with_disable_list(r#""langfuse_otel""#, &entries);
    let (_, names) = request_slot_scope(async {
        adapter
            .seed(&key, &disable_header("newrelic"))
            .expect("the key seeds");
        callback_names(&slot(&runtime))
    })
    .await;
    assert_eq!(names, vec!["langfuse_otel".to_owned()]);
}

async fn team_list_is_ignored() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let _fan_out = attach(&runtime, &sinks);
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let team = r#"{"litellm_disabled_callbacks": ["langfuse_otel"]}"#;
    let key = build_key_with_team(&logging(&langfuse_entry(TENANT_A, "pk-a", "sk-a")), team);
    let (_, names) = request_slot_scope(async {
        adapter.seed(&key, &no_headers()).expect("the key seeds");
        callback_names(&slot(&runtime))
    })
    .await;
    assert_eq!(names, vec!["langfuse_otel".to_owned()]);
}

async fn operator_switch_false() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let _fan_out = attach(&runtime, &sinks);
    let mut settings = setup(allowed_hosts(&[TENANT_A]));
    settings.callback_disabling = false;
    let adapter = build_adapter(&runtime, settings);
    let entries = [
        langfuse_entry(TENANT_A, "pk-a", "sk-a"),
        newrelic_entry("nr-a"),
    ]
    .join(",");
    let key = build_key_with_disable_list(r#""langfuse_otel""#, &entries);
    let (_, names) = request_slot_scope(async {
        adapter
            .seed(&key, &disable_header("newrelic"))
            .expect("the key seeds");
        callback_names(&slot(&runtime))
    })
    .await;
    assert_eq!(
        names,
        vec!["langfuse_otel".to_owned(), "newrelic".to_owned()]
    );
}

#[tokio::test]
async fn callback_disabling() {
    key_list_disables().await;
    header_replaces_key_list().await;
    team_list_is_ignored().await;
    operator_switch_false().await;
}
