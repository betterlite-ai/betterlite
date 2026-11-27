use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::registry::phases::CallbackSelection;
use rstest::rstest;

use super::support::{Harness, get_request, internal_key, json, master_key, object};
use crate::callback_config::ports::RuntimeCallback;
use crate::config_overlay_port::ConfigOverlayPortError;

const CALLBACKS_URI: &str = "/get/config/callbacks";

fn selection(success: &[&str], failure: &[&str], both: &[&str]) -> CallbackSelection {
    CallbackSelection {
        success: success.iter().map(|name| (*name).to_owned()).collect(),
        failure: failure.iter().map(|name| (*name).to_owned()).collect(),
        success_and_failure: both.iter().map(|name| (*name).to_owned()).collect(),
    }
}

fn runtime(name: &str, module: &str, type_name: &str, class_str: Option<&str>) -> RuntimeCallback {
    RuntimeCallback {
        name: name.to_owned(),
        module: module.to_owned(),
        type_name: type_name.to_owned(),
        class_str: class_str.map(str::to_owned),
        guardrail: false,
        registered_integration: false,
    }
}

fn rows(body: &JsonValue) -> Vec<JsonValue> {
    body["callbacks"].as_array().expect("callback rows").clone()
}

fn runtime_only(rows: &[JsonValue]) -> Vec<JsonValue> {
    rows.iter()
        .filter(|row| row.get("read_only").is_some())
        .cloned()
        .collect()
}

fn litellm_config(settings: &str) -> BTreeMap<String, JsonValue> {
    object(&[("litellm_settings", json(settings))])
}

async fn get_callbacks(harness: &Harness, caller: Option<Principal>) -> (StatusCode, JsonValue) {
    harness.call(get_request(CALLBACKS_URI), caller).await
}

async fn runtime_rows_for(
    configured: &str,
    classes: &[(&str, &str)],
    callback: RuntimeCallback,
) -> Vec<JsonValue> {
    let mut harness = Harness::new(litellm_config(&format!(
        r#"{{"success_callback": {configured}}}"#
    )));
    harness.callbacks.selection = selection(&[callback.name.as_str()], &[], &[]);
    harness.callbacks.classes = classes
        .iter()
        .map(|(name, class)| ((*name).to_owned(), (*class).to_owned()))
        .collect();
    harness.callbacks.objects = vec![callback];
    let (status, body) = get_callbacks(&harness, Some(master_key())).await;
    assert_eq!(status, StatusCode::OK);
    runtime_only(&rows(&body))
}

#[tokio::test]
async fn lists_the_configured_callbacks_with_their_variables() {
    let config = object(&[
        (
            "litellm_settings",
            json(
                r#"{"success_callback": ["langfuse"], "failure_callback": ["s3_v2"], "callbacks": ["galileo"]}"#,
            ),
        ),
        (
            "environment_variables",
            json(
                r#"{"LANGFUSE_PUBLIC_KEY": "pk", "GALILEO_USERNAME": "user", "S3_LOG_PROMPTS_ONLY": "True"}"#,
            ),
        ),
    ]);
    let harness = Harness::new(config).with_environment(&[("LANGFUSE_SECRET_KEY", "sk")]);
    let (status, body) = get_callbacks(&harness, Some(master_key())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], json(r#""success""#));
    let rows = rows(&body);
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0],
        json(
            r#"{"name": "langfuse", "variables": {"LANGFUSE_HOST": null, "LANGFUSE_PUBLIC_KEY": "pk", "LANGFUSE_SECRET_KEY": "sk"}, "type": "success"}"#
        )
    );
    assert_eq!(
        rows[1],
        json(
            r#"{"name": "s3_v2", "variables": {"AWS_ACCESS_KEY_ID": null, "AWS_REGION_NAME": null, "AWS_SECRET_ACCESS_KEY": null, "S3_LOG_PROMPTS_ONLY": "True", "S3_PARTITION_GRANULARITY": null}, "type": "failure"}"#
        )
    );
    assert_eq!(
        rows[2],
        json(
            r#"{"name": "galileo", "variables": {"GALILEO_API_KEY": null, "GALILEO_BASE_URL": null, "GALILEO_LOG_STREAM_ID": null, "GALILEO_PASSWORD": null, "GALILEO_PROJECT_ID": null, "GALILEO_USERNAME": "user"}, "type": "success_and_failure"}"#
        )
    );
}

#[tokio::test]
async fn stored_null_falls_back_to_the_process_and_an_empty_string_wins() {
    let config = object(&[
        (
            "litellm_settings",
            json(r#"{"success_callback": ["langfuse"]}"#),
        ),
        (
            "environment_variables",
            json(r#"{"LANGFUSE_PUBLIC_KEY": null, "LANGFUSE_HOST": ""}"#),
        ),
    ]);
    let harness = Harness::new(config).with_environment(&[
        ("LANGFUSE_PUBLIC_KEY", "env-pk"),
        ("LANGFUSE_HOST", "env-host"),
    ]);
    let (_status, body) = get_callbacks(&harness, Some(master_key())).await;
    let variables = &rows(&body)[0]["variables"];
    assert_eq!(variables["LANGFUSE_PUBLIC_KEY"], json(r#""env-pk""#));
    assert_eq!(variables["LANGFUSE_HOST"], json(r#""""#));
    assert_eq!(variables["LANGFUSE_SECRET_KEY"], json("null"));
}

#[tokio::test]
async fn reads_a_string_as_one_entry_and_null_as_none() {
    let harness = Harness::new(litellm_config(
        r#"{"success_callback": "langfuse", "failure_callback": null, "callbacks": "s3_v2"}"#,
    ));
    let (_status, body) = get_callbacks(&harness, Some(master_key())).await;
    let rows = rows(&body);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["name"], json(r#""langfuse""#));
    assert_eq!(rows[0]["type"], json(r#""success""#));
    assert_eq!(rows[1]["name"], json(r#""s3_v2""#));
    assert_eq!(rows[1]["type"], json(r#""success_and_failure""#));
}

#[tokio::test]
async fn adds_the_runtime_callbacks_normalized_and_read_only() {
    let mut harness =
        Harness::new(BTreeMap::new()).with_environment(&[("OTEL_ENDPOINT", "http://collector")]);
    harness.callbacks.selection = selection(&["opentelemetry"], &[], &[]);
    let (status, body) = get_callbacks(&harness, Some(master_key())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        rows(&body),
        vec![json(
            r#"{"name": "otel", "variables": {"OTEL_ENDPOINT": "http://collector", "OTEL_EXPORTER": null, "OTEL_EXPORTER_OTLP_PROTOCOL": null, "OTEL_HEADERS": null, "OTEL_TRACES_ENDPOINT": null}, "type": "success", "read_only": true}"#
        )]
    );
}

#[tokio::test]
async fn a_configured_alias_hides_the_runtime_row_it_names() {
    let mut harness = Harness::new(litellm_config(r#"{"success_callback": ["opentelemetry"]}"#));
    harness.callbacks.selection = selection(&["otel"], &[], &[]);
    let (_status, body) = get_callbacks(&harness, Some(master_key())).await;
    let rows = rows(&body);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], json(r#""opentelemetry""#));
    assert!(runtime_only(&rows).is_empty());
}

#[tokio::test]
async fn sorts_the_runtime_rows_by_name_and_type() {
    let mut harness = Harness::new(BTreeMap::new());
    harness.callbacks.selection = selection(&["bravo", "alpha"], &["alpha"], &["alpha"]);
    let (_status, body) = get_callbacks(&harness, Some(master_key())).await;
    let seen: Vec<(String, String)> = rows(&body)
        .iter()
        .map(|row| {
            (
                row["name"].as_str().expect("a name").to_owned(),
                row["type"].as_str().expect("a type").to_owned(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        vec![
            ("alpha".to_owned(), "failure".to_owned()),
            ("alpha".to_owned(), "success".to_owned()),
            ("alpha".to_owned(), "success_and_failure".to_owned()),
            ("bravo".to_owned(), "success".to_owned()),
        ]
    );
}

#[tokio::test]
async fn hides_the_static_guardrail_and_internal_runtime_callbacks() {
    assert!(
        runtime_rows_for(
            "[]",
            &[],
            runtime("cache", "litellm.caching", "Cache", None)
        )
        .await
        .is_empty()
    );
    assert!(
        runtime_rows_for(
            "[]",
            &[],
            runtime(
                "vector_store_pre_call_hook",
                "litellm.integrations.vector_store_integrations.vector_store_pre_call_hook",
                "VectorStorePreCallHook",
                None
            )
        )
        .await
        .is_empty()
    );
    assert!(
        runtime_rows_for(
            "[]",
            &[],
            RuntimeCallback {
                guardrail: true,
                ..runtime("custom_guard", "user_module", "CustomGuard", None)
            }
        )
        .await
        .is_empty()
    );
    assert!(
        runtime_rows_for(
            "[]",
            &[],
            runtime(
                "internal_logger",
                "litellm.proxy.hooks.internal",
                "Internal",
                None
            )
        )
        .await
        .is_empty()
    );
    let registered = runtime_rows_for(
        "[]",
        &[],
        RuntimeCallback {
            registered_integration: true,
            ..runtime(
                "otel",
                "litellm.integrations.opentelemetry",
                "OpenTelemetry",
                Some("opentelemetry"),
            )
        },
    )
    .await;
    assert_eq!(registered.len(), 1);
    assert_eq!(registered[0]["name"], json(r#""otel""#));
}

#[tokio::test]
async fn hides_the_runtime_callbacks_a_configured_name_claims() {
    assert!(
        runtime_rows_for(
            r#"["signoz"]"#,
            &[("signoz", "opentelemetry")],
            runtime(
                "OpenTelemetry",
                "litellm.integrations.opentelemetry",
                "OpenTelemetry",
                Some("opentelemetry"),
            )
        )
        .await
        .is_empty()
    );
    assert!(
        runtime_rows_for(
            r#"["signoz"]"#,
            &[("signoz", "opentelemetry")],
            runtime(
                "opentelemetry",
                "user_module",
                "OpenTelemetry",
                Some("opentelemetry"),
            )
        )
        .await
        .is_empty()
    );
    let sibling = runtime_rows_for(
        r#"["logfire"]"#,
        &[("logfire", "opentelemetry")],
        RuntimeCallback {
            registered_integration: true,
            ..runtime(
                "arize",
                "litellm.integrations.opentelemetry",
                "OpenTelemetry",
                Some("opentelemetry"),
            )
        },
    )
    .await;
    assert_eq!(sibling.len(), 1);
    assert_eq!(sibling[0]["name"], json(r#""arize""#));
    assert!(
        runtime_rows_for(
            r#"["my_module.my_callback"]"#,
            &[],
            runtime("my_callback", "my_module", "my_callback", None)
        )
        .await
        .is_empty()
    );
    let elsewhere = runtime_rows_for(
        r#"["my_module.my_callback"]"#,
        &[],
        runtime("my_callback", "other_module", "my_callback", None),
    )
    .await;
    assert_eq!(elsewhere.len(), 1);
}

#[tokio::test]
async fn redacts_sensitive_variables_for_everyone_but_a_proxy_admin() {
    let config = object(&[
        (
            "litellm_settings",
            json(r#"{"success_callback": ["langfuse", "galileo"]}"#),
        ),
        (
            "environment_variables",
            json(
                r#"{"LANGFUSE_PUBLIC_KEY": "pk", "LANGFUSE_SECRET_KEY": "sk", "LANGFUSE_HOST": "https://langfuse", "GALILEO_USERNAME": "user"}"#,
            ),
        ),
    ]);
    let harness = Harness::new(config).with_environment(&[("GALILEO_PASSWORD", "secret")]);
    let (_status, admin_body) = get_callbacks(&harness, Some(master_key())).await;
    let admin_rows = rows(&admin_body);
    assert_eq!(
        admin_rows[0]["variables"]["LANGFUSE_SECRET_KEY"],
        json(r#""sk""#)
    );
    assert_eq!(
        admin_rows[1]["variables"]["GALILEO_USERNAME"],
        json(r#""user""#)
    );
    let (_status, viewer_body) = get_callbacks(&harness, Some(internal_key())).await;
    let viewer_rows = rows(&viewer_body);
    assert_eq!(
        viewer_rows[0]["variables"],
        json(
            r#"{"LANGFUSE_HOST": "https://langfuse", "LANGFUSE_PUBLIC_KEY": "REDACTED", "LANGFUSE_SECRET_KEY": "REDACTED"}"#
        )
    );
    assert_eq!(
        viewer_rows[1]["variables"]["GALILEO_USERNAME"],
        json(r#""REDACTED""#)
    );
    assert_eq!(
        viewer_rows[1]["variables"]["GALILEO_PASSWORD"],
        json(r#""REDACTED""#)
    );
    assert_eq!(viewer_rows[1]["variables"]["GALILEO_API_KEY"], json("null"));
}

#[tokio::test]
async fn reads_router_settings_from_the_router_when_one_is_mounted() {
    let without = Harness::new(BTreeMap::new());
    let (_status, body) = get_callbacks(&without, Some(master_key())).await;
    assert_eq!(body["router_settings"], json("{}"));
    let with = Harness::new(BTreeMap::new()).with_router(object(&[
        ("num_retries", json("3")),
        ("timeout", json("30")),
    ]));
    let (_status, body) = get_callbacks(&with, Some(master_key())).await;
    assert_eq!(
        body["router_settings"],
        json(r#"{"num_retries": 3, "timeout": 30}"#)
    );
}

#[tokio::test]
async fn lists_the_available_callbacks_litellm_documents() {
    let harness = Harness::new(BTreeMap::new());
    let (_status, body) = get_callbacks(&harness, Some(master_key())).await;
    let available = body["available_callbacks"].as_object().expect("the table");
    assert_eq!(available.len(), 17);
    assert_eq!(
        available["langfuse"],
        json(
            r#"{"litellm_callback_name": "langfuse", "litellm_callback_params": ["LANGFUSE_PUBLIC_KEY", "LANGFUSE_SECRET_KEY", "LANGFUSE_HOST"], "ui_callback_name": "Langfuse"}"#
        )
    );
    assert_eq!(
        available["signoz"],
        json(
            r#"{"litellm_callback_name": "signoz", "litellm_callback_params": ["SIGNOZ_INGESTION_ENDPOINT", "SIGNOZ_INGESTION_KEY"], "ui_callback_name": "SigNoz"}"#
        )
    );
    assert_eq!(
        available["custom_callback_api"],
        json(
            r#"{"litellm_callback_name": "custom_callback_api", "litellm_callback_params": ["GENERIC_LOGGER_ENDPOINT", "GENERIC_LOGGER_HEADERS"], "ui_callback_name": "Custom Callback API"}"#
        )
    );
}

#[tokio::test]
async fn answers_an_auth_error_when_the_reload_fails() {
    let harness = Harness::new(BTreeMap::new()).failing_reload(ConfigOverlayPortError::Reload);
    let (status, body) = get_callbacks(&harness, Some(master_key())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json(
            r#"{"error": {"message": "Authentication Error, the config could not be reloaded", "type": "auth_error", "param": "None", "code": "400"}}"#
        )
    );
}

#[tokio::test]
async fn requires_authentication() {
    let harness = Harness::new(BTreeMap::new());
    let (status, _body) = get_callbacks(&harness, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(harness.reload_calls(), 0);
}

#[rstest]
#[case(r#"{"success_callback": {"langfuse": true}}"#)]
#[case(r#"{"success_callback": [5]}"#)]
#[tokio::test]
async fn names_the_field_when_a_callback_list_is_malformed(#[case] settings: &str) {
    let harness = Harness::new(litellm_config(settings));
    let (status, body) = get_callbacks(&harness, Some(master_key())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["error"]["message"],
        json(
            r#""Authentication Error, litellm_settings.success_callback must be a list of strings""#
        )
    );
}
