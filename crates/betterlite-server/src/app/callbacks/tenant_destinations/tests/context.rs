use betterlite_auth::auth_admission::ports::TenantDestinations as _;
use betterlite_logging::otel::proxy_runtime::slot::{request_slot_scope, set_destinations};
use opentelemetry_sdk::trace::InMemorySpanExporter;

use super::support::{
    NEWRELIC_US_ENDPOINT, OPERATOR_ENDPOINT, RecordingSinks, TENANT_A, allowed_hosts, attach,
    authorization_pair, build_adapter, build_auth, build_key, build_provider, build_runtime,
    callback_names, end_database_span, end_span, endpoints, finished, flush, headers_of,
    langfuse_endpoint, langfuse_entry, logging, newrelic_entry, no_headers, operator_filter,
    operator_provider, prior_destination, setup, slot,
};

#[tokio::test]
async fn successful_empty_clears_context() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let _fan_out = attach(&runtime, &sinks);
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let key = build_key(r#"{"logging": []}"#);
    let (_, names) = request_slot_scope(async {
        set_destinations(vec![prior_destination()]);
        adapter.seed(&key, &no_headers()).expect("the key seeds");
        callback_names(&slot(&runtime))
    })
    .await;
    assert!(names.is_empty());
}

#[tokio::test]
async fn resolution_error_keeps_auth_running() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let _fan_out = attach(&runtime, &sinks);
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let entries = langfuse_entry(TENANT_A, "pk-a", "sk-a");
    let broken = build_auth(&format!(
        r#"{{"api_key": "sk-test", "budget_reset_at": "2024-01-01T00:00:00", "metadata": {{"logging": [{entries}]}}}}"#
    ));
    let key = build_key(&logging(&entries));
    let (_, outcome) = request_slot_scope(async {
        set_destinations(vec![prior_destination()]);
        let fault = adapter
            .seed(&broken, &no_headers())
            .expect_err("the key cannot be read");
        let kept = endpoints(&slot(&runtime));
        adapter
            .seed(&key, &no_headers())
            .expect("auth keeps running");
        (fault.to_string(), kept, endpoints(&slot(&runtime)))
    })
    .await;
    let (fault, kept, after) = outcome;
    assert!(fault.starts_with("the authenticated key cannot be read for tenant destinations"));
    assert_eq!(kept, vec![OPERATOR_ENDPOINT.to_owned()]);
    assert_eq!(after, vec![langfuse_endpoint(TENANT_A)]);
}

#[tokio::test]
async fn auth_span_inherits_destinations() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let fan_out = attach(&runtime, &sinks);
    let provider = build_provider(fan_out.clone());
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let key = build_key(&logging(&langfuse_entry(TENANT_A, "pk-a", "sk-a")));
    request_slot_scope(async {
        adapter.seed(&key, &no_headers()).expect("the key seeds");
        end_span(&provider, "auth");
    })
    .await;
    flush(&fan_out);
    let spans = sinks.spans(&langfuse_endpoint(TENANT_A));
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name.as_ref(), "auth");
}

#[tokio::test]
async fn earlier_database_span_keeps_operator() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let fan_out = attach(&runtime, &sinks);
    let operator = InMemorySpanExporter::default();
    let provider = operator_provider(operator_filter(&runtime, &operator), &fan_out);
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let key = build_key(&logging(&langfuse_entry(TENANT_A, "pk-a", "sk-a")));
    request_slot_scope(async {
        end_database_span(&provider);
        adapter.seed(&key, &no_headers()).expect("the key seeds");
        end_span(&provider, "llm");
    })
    .await;
    flush(&fan_out);
    let kept = operator
        .get_finished_spans()
        .expect("the operator keeps its spans");
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].name.as_ref(), "db_lookup");
    assert_eq!(sinks.spans(&langfuse_endpoint(TENANT_A)).len(), 1);
}

#[tokio::test]
async fn concurrent_tenants_are_isolated() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let fan_out = attach(&runtime, &sinks);
    let operator = InMemorySpanExporter::default();
    let provider = operator_provider(operator_filter(&runtime, &operator), &fan_out);
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let key_a = build_key(&logging(&langfuse_entry(TENANT_A, "pk-a", "sk-a")));
    let key_b = build_key(&logging(&newrelic_entry("nr-b")));
    let request_a = request_slot_scope(async {
        adapter.seed(&key_a, &no_headers()).expect("tenant a seeds");
        tokio::task::yield_now().await;
        end_span(&provider, "llm-a");
        let destinations = slot(&runtime);
        (endpoints(&destinations), authorization_pair(&destinations))
    });
    let request_b = request_slot_scope(async {
        adapter.seed(&key_b, &no_headers()).expect("tenant b seeds");
        tokio::task::yield_now().await;
        end_span(&provider, "llm-b");
        endpoints(&slot(&runtime))
    });
    let (result_a, result_b) = tokio::join!(request_a, request_b);
    flush(&fan_out);
    let (_, (ends_a, auth_a)) = result_a;
    let (_, ends_b) = result_b;
    assert_tenant_isolation(&sinks, &operator, ends_a, ends_b, &auth_a);
}

fn assert_tenant_isolation(
    sinks: &RecordingSinks,
    operator: &InMemorySpanExporter,
    ends_a: Vec<String>,
    ends_b: Vec<String>,
    auth_a: &str,
) {
    assert_eq!(ends_a, vec![langfuse_endpoint(TENANT_A)]);
    assert_eq!(ends_b, vec![NEWRELIC_US_ENDPOINT.to_owned()]);
    assert_eq!(sinks.spans(&langfuse_endpoint(TENANT_A)).len(), 1);
    assert_eq!(sinks.spans(NEWRELIC_US_ENDPOINT).len(), 1);
    let gathered = operator
        .get_finished_spans()
        .expect("the operator keeps its spans");
    assert_eq!(gathered.len(), 1);
    assert_eq!(gathered[0].name.as_ref(), "llm-b");
    assert_eq!(finished(operator), 1);
    let headers_a = headers_of(sinks, &langfuse_endpoint(TENANT_A));
    assert!(headers_a.contains(auth_a));
    assert!(!headers_a.contains("nr-b"));
    let headers_b = headers_of(sinks, NEWRELIC_US_ENDPOINT);
    assert!(headers_b.contains("nr-b"));
    assert!(!headers_b.contains(auth_a));
}
