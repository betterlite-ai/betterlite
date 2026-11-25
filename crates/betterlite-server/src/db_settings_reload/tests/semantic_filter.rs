use std::sync::atomic::Ordering;

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::support::{
    ControlLog, FakeFilter, FakeRows, RebuildCall, TestRouter, capture_logs, json, reloader,
};
use crate::db_settings_reload::ports::{SemanticFilterConfig, SemanticFilterReload};

fn rows_with(config: &str) -> FakeRows {
    FakeRows::litellm(json(&format!("{{\"mcp_semantic_tool_filter\":{config}}}")))
}

fn port(filter: &FakeFilter) -> &dyn SemanticFilterReload<TestRouter> {
    filter
}

fn filter_config(enabled: bool) -> SemanticFilterConfig {
    SemanticFilterConfig {
        enabled,
        embedding_model: None,
        top_k: None,
        similarity_threshold: None,
    }
}

#[tokio::test]
async fn an_absent_row_does_nothing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&FakeRows::default(), Some(port(&filter)), None)
        .await;
    assert!(filter.rebuilds().is_empty());
    assert_eq!(logs.contents(), "");
}

#[tokio::test]
async fn an_absent_key_does_nothing() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    let rows = FakeRows::litellm(json("{\"websearch_interception_params\":{}}"));
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&rows, Some(port(&filter)), None)
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
        .reload_semantic_filter(&rows_with("null"), Some(port(&filter)), None)
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
        .reload_semantic_filter(&rows_with("5"), Some(port(&filter)), None)
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
    let router = TestRouter;
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(
            &rows_with("{\"enabled\":false}"),
            Some(port(&filter)),
            Some(&router),
        )
        .await;
    assert_eq!(
        filter.rebuilds(),
        vec![RebuildCall {
            config: filter_config(false),
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
        .reload_semantic_filter(&rows_with("{}"), Some(port(&filter)), None)
        .await;
    assert_eq!(
        filter.rebuilds(),
        vec![RebuildCall {
            config: filter_config(false),
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
        .reload_semantic_filter(&rows_with("{\"enabled\":null}"), Some(port(&filter)), None)
        .await;
    assert_eq!(
        filter.rebuilds(),
        vec![RebuildCall {
            config: filter_config(false),
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
    let router = TestRouter;
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(
            &rows_with("{\"enabled\":true}"),
            Some(port(&filter)),
            Some(&router),
        )
        .await;
    assert_eq!(
        filter.rebuilds(),
        vec![RebuildCall {
            config: filter_config(true),
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
    let router = TestRouter;
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(
            &rows_with("{\"enabled\":true}"),
            Some(port(&filter)),
            Some(&router),
        )
        .await;
    assert_eq!(
        filter.rebuilds(),
        vec![RebuildCall {
            config: filter_config(true),
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
        .reload_semantic_filter(&rows_with("{\"enabled\":false}"), Some(port(&filter)), None)
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
        .reload_semantic_filter(&rows_with("{\"enabled\":false}"), Some(port(&filter)), None)
        .await;
    assert_eq!(
        filter.rebuilds(),
        vec![RebuildCall {
            config: filter_config(false),
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
        .reload_semantic_filter(&rows_with("{\"enabled\":false}"), Some(port(&filter)), None)
        .await;
    assert_eq!(filter.rebuilds().len(), 1);
    assert!(!logs.contents().contains("unchanged"));
}

#[tokio::test]
async fn the_stored_model_and_ranking_settings_reach_the_rebuild() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    let (_logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(
            &rows_with(
                "{\"enabled\":true,\"embedding_model\":\"custom-embedding\",\"top_k\":3,\"similarity_threshold\":0.75}",
            ),
            Some(port(&filter)),
            None,
        )
        .await;
    assert_eq!(
        filter.rebuilds(),
        vec![RebuildCall {
            config: SemanticFilterConfig {
                enabled: true,
                embedding_model: Some("custom-embedding".to_owned()),
                top_k: Some(3),
                similarity_threshold: Some(0.75),
            },
            has_router: false,
        }]
    );
}

#[tokio::test]
async fn a_change_to_a_ranking_field_rebuilds_with_the_new_value() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    filter.ready.store(true, Ordering::SeqCst);
    let (_logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(
            &rows_with("{\"enabled\":true,\"top_k\":5}"),
            Some(port(&filter)),
            None,
        )
        .await;
    reloader
        .reload_semantic_filter(
            &rows_with("{\"enabled\":true,\"top_k\":3}"),
            Some(port(&filter)),
            None,
        )
        .await;
    let rebuilds = filter.rebuilds();
    assert_eq!(rebuilds.len(), 2);
    assert_eq!(rebuilds[0].config.top_k, Some(5));
    assert_eq!(rebuilds[1].config.top_k, Some(3));
}

#[rstest]
#[case::embedding_model(
    "{\"embedding_model\":5}",
    "mcp_semantic_tool_filter.embedding_model must be a string"
)]
#[case::top_k(
    "{\"top_k\":\"3\"}",
    "mcp_semantic_tool_filter.top_k must be an integer"
)]
#[case::top_k_float("{\"top_k\":3.0}", "mcp_semantic_tool_filter.top_k must be an integer")]
#[case::similarity_threshold(
    "{\"similarity_threshold\":\"high\"}",
    "mcp_semantic_tool_filter.similarity_threshold must be a number"
)]
#[tokio::test]
async fn a_filter_field_of_another_type_is_an_error(#[case] stored: &str, #[case] message: &str) {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&rows_with(stored), Some(port(&filter)), None)
        .await;
    assert!(filter.rebuilds().is_empty());
    assert!(logs.contents().contains(&format!(
        "Error initializing semantic filter settings from DB: {message}"
    )));
    assert_eq!(reloader.last_semantic_filter, None);
}

#[tokio::test]
async fn an_enabled_that_is_not_a_bool_is_an_error() {
    let control = ControlLog::default();
    let mut reloader = reloader(&control);
    let filter = FakeFilter::default();
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(
            &rows_with("{\"enabled\":\"true\"}"),
            Some(port(&filter)),
            None,
        )
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
    let rows = FakeRows::litellm(JsonValue::String("[]".to_owned()));
    let (logs, _guard) = capture_logs();
    reloader
        .reload_semantic_filter(&rows, Some(port(&filter)), None)
        .await;
    assert!(filter.rebuilds().is_empty());
    assert!(logs.contents().contains(
        "Error initializing semantic filter settings from DB: litellm_settings is not a JSON object"
    ));
}
