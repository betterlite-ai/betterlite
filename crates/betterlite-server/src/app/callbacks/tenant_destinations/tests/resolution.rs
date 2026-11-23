use betterlite_auth::auth_admission::ports::TenantDestinations as _;
use betterlite_logging::otel::proxy_runtime::slot::{request_slot_scope, set_destinations};
use opentelemetry_sdk::trace::InMemorySpanExporter;

use super::support::{
    NEWRELIC_US_ENDPOINT, RecordingSinks, TENANT_A, UNBUILDABLE_HOST, allowed_hosts, attach,
    authorization, basic, build_adapter, build_key, build_key_with_team, build_runtime,
    callback_names, end_span, endpoints, failure_langfuse_entry, finished, flush,
    langfuse_endpoint, langfuse_entry, langfuse_host_entry, logging, newrelic_entry, no_headers,
    operator_filter, operator_provider, prior_destination, setup, slot,
};

#[tokio::test]
async fn v2_off() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let _fan_out = attach(&runtime, &sinks);
    let mut settings = setup(allowed_hosts(&[TENANT_A]));
    settings.otel_v2_enabled = false;
    let adapter = build_adapter(&runtime, settings);
    let key = build_key(&logging(&langfuse_entry(TENANT_A, "pk-a", "sk-a")));
    let (_, names) = request_slot_scope(async {
        set_destinations(vec![prior_destination()]);
        adapter.seed(&key, &no_headers()).expect("the key seeds");
        callback_names(&slot(&runtime))
    })
    .await;
    assert!(names.is_empty());
}

#[tokio::test]
async fn key_empty_blocks_team() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let _fan_out = attach(&runtime, &sinks);
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let team = logging(&langfuse_entry(TENANT_A, "pk-a", "sk-a"));
    let key = build_key_with_team(r#"{"logging": []}"#, &team);
    let (_, names) = request_slot_scope(async {
        set_destinations(vec![prior_destination()]);
        adapter.seed(&key, &no_headers()).expect("the key seeds");
        callback_names(&slot(&runtime))
    })
    .await;
    assert!(names.is_empty());
}

#[tokio::test]
async fn team_fallback() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let _fan_out = attach(&runtime, &sinks);
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let team = logging(&langfuse_entry(TENANT_A, "pk-a", "sk-a"));
    let key = build_key_with_team("{}", &team);
    let (_, delivered) = request_slot_scope(async {
        adapter.seed(&key, &no_headers()).expect("the key seeds");
        endpoints(&slot(&runtime))
    })
    .await;
    assert_eq!(delivered, vec![langfuse_endpoint(TENANT_A)]);
}

#[tokio::test]
async fn last_callback_vars_win() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let _fan_out = attach(&runtime, &sinks);
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let entries = format!(
        "{},{}",
        langfuse_entry(TENANT_A, "pk-1", "sk-1"),
        langfuse_entry(TENANT_A, "pk-2", "sk-2")
    );
    let key = build_key(&logging(&entries));
    let (_, destinations) = request_slot_scope(async {
        adapter.seed(&key, &no_headers()).expect("the key seeds");
        slot(&runtime)
    })
    .await;
    assert_eq!(endpoints(&destinations), vec![langfuse_endpoint(TENANT_A)]);
    assert_eq!(
        authorization(&destinations[0]),
        Some(basic("pk-2", "sk-2").as_str())
    );
}

#[tokio::test]
async fn failure_only_is_not_eligible() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let _fan_out = attach(&runtime, &sinks);
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let key = build_key(&logging(&failure_langfuse_entry("pk-f", "sk-f")));
    let (_, names) = request_slot_scope(async {
        adapter.seed(&key, &no_headers()).expect("the key seeds");
        callback_names(&slot(&runtime))
    })
    .await;
    assert!(names.is_empty());
}

#[tokio::test]
async fn failure_vars_merge_into_eligible_backend() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let _fan_out = attach(&runtime, &sinks);
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let entries = format!(
        "{},{}",
        langfuse_host_entry(TENANT_A),
        failure_langfuse_entry("pk-a", "sk-a")
    );
    let key = build_key(&logging(&entries));
    let (_, destinations) = request_slot_scope(async {
        adapter.seed(&key, &no_headers()).expect("the key seeds");
        slot(&runtime)
    })
    .await;
    assert_eq!(endpoints(&destinations), vec![langfuse_endpoint(TENANT_A)]);
    assert_eq!(
        authorization(&destinations[0]),
        Some(basic("pk-a", "sk-a").as_str())
    );
}

#[tokio::test]
async fn incomplete_backend_names_no_destination() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let _fan_out = attach(&runtime, &sinks);
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let partial = format!(
        r#"{{"callback_name": "langfuse_otel", "callback_vars": {{"langfuse_host": "{TENANT_A}", "langfuse_public_key": "pk-a"}}}}"#
    );
    let entries = format!("{partial},{}", newrelic_entry("nr-a"));
    let key = build_key(&logging(&entries));
    let (_, destinations) = request_slot_scope(async {
        adapter.seed(&key, &no_headers()).expect("the key seeds");
        slot(&runtime)
    })
    .await;
    assert_eq!(
        endpoints(&destinations),
        vec![NEWRELIC_US_ENDPOINT.to_owned()]
    );
}

#[tokio::test]
async fn unbuildable_destination_keeps_operator() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let fan_out = attach(&runtime, &sinks);
    let operator = InMemorySpanExporter::default();
    let provider = operator_provider(operator_filter(&runtime, &operator), &fan_out);
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[UNBUILDABLE_HOST])));
    let entries = format!(
        "{},{}",
        langfuse_entry(UNBUILDABLE_HOST, "pk-u", "sk-u"),
        newrelic_entry("nr-a")
    );
    let key = build_key(&logging(&entries));
    let (_, names) = request_slot_scope(async {
        adapter.seed(&key, &no_headers()).expect("the key seeds");
        end_span(&provider, "llm");
        callback_names(&slot(&runtime))
    })
    .await;
    flush(&fan_out);
    assert_eq!(names, vec!["newrelic".to_owned()]);
    assert_eq!(finished(&operator), 1);
    assert_eq!(sinks.spans(NEWRELIC_US_ENDPOINT).len(), 1);
    assert!(
        sinks
            .headers(&langfuse_endpoint(UNBUILDABLE_HOST))
            .is_none()
    );
}

#[tokio::test]
async fn provider_published_after_construction() {
    let runtime = build_runtime();
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let key = build_key(&logging(&langfuse_entry(TENANT_A, "pk-a", "sk-a")));
    let (_, before) = request_slot_scope(async {
        adapter.seed(&key, &no_headers()).expect("the key seeds");
        endpoints(&slot(&runtime))
    })
    .await;
    assert!(before.is_empty());
    let sinks = RecordingSinks::new();
    let _fan_out = attach(&runtime, &sinks);
    let (_, after) = request_slot_scope(async {
        adapter.seed(&key, &no_headers()).expect("the key seeds");
        endpoints(&slot(&runtime))
    })
    .await;
    assert_eq!(after, vec![langfuse_endpoint(TENANT_A)]);
}
