use std::sync::atomic::Ordering;

use betterlite_config::sections::settings::JsonValue;

use super::support::{
    ControlLog, FakeFilter, FakeRows, RebuildCall, TestRouter, capture_logs, json, reloader,
};
use crate::db_settings_reload::ports::SemanticFilterReload;

fn rows_with(config: &str) -> FakeRows {
    FakeRows {
        litellm_settings: Some(json(&format!("{{\"mcp_semantic_tool_filter\":{config}}}"))),
        ..FakeRows::default()
    }
}

fn port(filter: &FakeFilter) -> Option<&dyn SemanticFilterReload<TestRouter>> {
    Some(filter)
}

#[tokio::test]
async fn an_absent_row_does_nothing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&FakeRows::default(), port(&filter), None)
        .await;
    assert!(filter.rebuilds().is_empty());
    assert_eq!(logs.contents(), "");
}

#[tokio::test]
async fn an_absent_key_does_nothing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    let rows = FakeRows {
        litellm_settings: Some(json("{\"websearch_interception_params\":{}}")),
        ..FakeRows::default()
    };
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&rows, port(&filter), None)
        .await;
    assert!(filter.rebuilds().is_empty());
    assert_eq!(logs.contents(), "");
}

#[tokio::test]
async fn a_null_value_does_nothing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&rows_with("null"), port(&filter), None)
        .await;
    assert!(filter.rebuilds().is_empty());
    assert_eq!(logs.contents(), "");
}

#[tokio::test]
async fn a_value_that_is_not_a_mapping_is_logged() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&rows_with("5"), port(&filter), None)
        .await;
    assert!(filter.rebuilds().is_empty());
    assert!(logs.contents().contains(
        "Error initializing semantic filter settings from DB: mcp_semantic_tool_filter is not a JSON object"
    ));
}

#[tokio::test]
async fn a_missing_port_logs_the_missing_filter_and_remembers_no_value() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let rows = rows_with("{\"enabled\":false}");
    let router: Option<&TestRouter> = None;
    let (logs, _guard) = capture_logs();
    reloader.reload_semantic_filter(&rows, None, router).await;
    reloader.reload_semantic_filter(&rows, None, router).await;
    assert_eq!(
        logs.count("no semantic filter is installed for 'mcp_semantic_tool_filter'"),
        2
    );
    assert_eq!(reloader.last_semantic_filter, None);
}

#[tokio::test]
async fn a_missing_port_logs_even_when_the_value_equals_the_last_applied_one() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    reloader.last_semantic_filter = Some(json("{\"enabled\":false}"));
    let router: Option<&TestRouter> = None;
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&rows_with("{\"enabled\":false}"), None, router)
        .await;
    assert_eq!(
        logs.count("no semantic filter is installed for 'mcp_semantic_tool_filter'"),
        1
    );
}

#[tokio::test]
async fn a_disabled_config_rebuilds_and_is_remembered() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    let router = TestRouter { name: "router" };
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(
            &rows_with("{\"enabled\":false}"),
            port(&filter),
            Some(&router),
        )
        .await;
    assert_eq!(
        filter.rebuilds(),
        vec![RebuildCall {
            enabled: false,
            has_router: true,
        }]
    );
    assert!(logs.contents().contains("MCP Semantic Filter disabled"));
    assert_eq!(
        reloader.last_semantic_filter,
        Some(json("{\"enabled\":false}"))
    );
}

#[tokio::test]
async fn a_config_without_enabled_is_rebuilt_as_disabled() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    let (_logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&rows_with("{}"), port(&filter), None)
        .await;
    assert_eq!(
        filter.rebuilds(),
        vec![RebuildCall {
            enabled: false,
            has_router: false,
        }]
    );
}

#[tokio::test]
async fn a_null_enabled_is_rebuilt_as_disabled() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    let (_logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&rows_with("{\"enabled\":null}"), port(&filter), None)
        .await;
    assert_eq!(
        filter.rebuilds(),
        vec![RebuildCall {
            enabled: false,
            has_router: false,
        }]
    );
}

#[tokio::test]
async fn an_enabled_config_that_registers_a_hook_says_so() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    filter.registers.store(true, Ordering::SeqCst);
    let router = TestRouter { name: "router" };
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(
            &rows_with("{\"enabled\":true}"),
            port(&filter),
            Some(&router),
        )
        .await;
    assert_eq!(
        filter.rebuilds(),
        vec![RebuildCall {
            enabled: true,
            has_router: true,
        }]
    );
    assert!(
        logs.contents()
            .contains("MCP Semantic Filter reinitialized from DB")
    );
    assert_eq!(
        reloader.last_semantic_filter,
        Some(json("{\"enabled\":true}"))
    );
}

#[tokio::test]
async fn an_enabled_config_that_registers_no_hook_says_nothing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    let router = TestRouter { name: "router" };
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(
            &rows_with("{\"enabled\":true}"),
            port(&filter),
            Some(&router),
        )
        .await;
    assert_eq!(
        filter.rebuilds(),
        vec![RebuildCall {
            enabled: true,
            has_router: true,
        }]
    );
    assert_eq!(logs.contents(), "");
    assert_eq!(
        reloader.last_semantic_filter,
        Some(json("{\"enabled\":true}"))
    );
}

#[tokio::test]
async fn an_unchanged_config_with_a_ready_hook_is_skipped() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    reloader.last_semantic_filter = Some(json("{\"enabled\":false}"));
    let filter = FakeFilter::default();
    filter.ready.store(true, Ordering::SeqCst);
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&rows_with("{\"enabled\":false}"), port(&filter), None)
        .await;
    assert!(filter.rebuilds().is_empty());
    assert!(
        logs.contents()
            .contains("Semantic filter settings unchanged, skipping reinitialization")
    );
}

#[tokio::test]
async fn an_unchanged_config_without_a_ready_hook_rebuilds() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    reloader.last_semantic_filter = Some(json("{\"enabled\":false}"));
    let filter = FakeFilter::default();
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&rows_with("{\"enabled\":false}"), port(&filter), None)
        .await;
    assert_eq!(
        filter.rebuilds(),
        vec![RebuildCall {
            enabled: false,
            has_router: false,
        }]
    );
    assert!(logs.contents().contains(
        "Semantic filter settings unchanged, but hook is missing or uninitialized. Reinitializing."
    ));
}

#[tokio::test]
async fn a_changed_config_rebuilds_even_with_a_ready_hook() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    reloader.last_semantic_filter = Some(json("{\"enabled\":true}"));
    let filter = FakeFilter::default();
    filter.ready.store(true, Ordering::SeqCst);
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&rows_with("{\"enabled\":false}"), port(&filter), None)
        .await;
    assert_eq!(filter.rebuilds().len(), 1);
    assert!(!logs.contents().contains("unchanged"));
}

#[tokio::test]
async fn an_enabled_that_is_not_a_bool_is_an_error() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&rows_with("{\"enabled\":\"true\"}"), port(&filter), None)
        .await;
    assert!(filter.rebuilds().is_empty());
    assert!(logs.contents().contains(
        "Error initializing semantic filter settings from DB: mcp_semantic_tool_filter.enabled must be a boolean"
    ));
    assert_eq!(reloader.last_semantic_filter, None);
}

#[tokio::test]
async fn a_litellm_settings_that_is_not_an_object_is_logged() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    let rows = FakeRows {
        litellm_settings: Some(JsonValue::String("[]".to_owned())),
        ..FakeRows::default()
    };
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&rows, port(&filter), None)
        .await;
    assert!(filter.rebuilds().is_empty());
    assert!(logs.contents().contains(
        "Error initializing semantic filter settings from DB: litellm_settings is not a JSON object"
    ));
}
