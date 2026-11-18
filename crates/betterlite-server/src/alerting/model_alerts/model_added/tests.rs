#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_batching::alert_delivery::PROXY_BASE_URL_ENV;
use betterlite_logging::alert_dispatch::AlertLevel;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::render_model_added_message;
use super::{ModelAddedAlertError, ModelAddedAlerts, ModelInfoRenderError, RawModelInfoSource};
use crate::alerting::model_alerts::tests::support::FakeSender;

struct FakeCatalog(IndexMap<String, JsonObject>);

impl RawModelInfoSource for FakeCatalog {
    fn model_info(&self, name: &str) -> Option<&JsonObject> {
        self.0.get(name)
    }
}

fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    let mut object = IndexMap::new();
    for (key, value) in entries {
        object.insert(key.to_owned(), value);
    }
    object
}

fn catalog(entries: Vec<(&str, JsonObject)>) -> FakeCatalog {
    let mut catalog = IndexMap::new();
    for (name, entry) in entries {
        catalog.insert(name.to_owned(), entry);
    }
    FakeCatalog(catalog)
}

fn float(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("a finite test number"))
}

fn integer(value: u64) -> Json {
    Json::Number(Number::from(value))
}

fn environment(base_url: Option<&str>) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    if let Some(base_url) = base_url {
        environment.insert(PROXY_BASE_URL_ENV.to_owned(), base_url.to_owned());
    }
    environment
}

fn alerts(
    catalog: FakeCatalog,
    sender: FakeSender,
    environment: BTreeMap<String, String>,
) -> ModelAddedAlerts<FakeCatalog, FakeSender, BTreeMap<String, String>> {
    ModelAddedAlerts {
        catalog,
        sender,
        environment,
    }
}

fn expected_message(model_name: &str, base_line: &str, model_info: &str, base_url: &str) -> String {
    format!(
        "\n*\u{1f685} New Model Added*\nModel Name: `{model_name}`\n{base_line}\n\nUsage OpenAI Python SDK:\n```\nimport openai\nclient = openai.OpenAI(\n    api_key=\"your_api_key\",\n    base_url={base_url}\n)\n\nresponse = client.chat.completions.create(\n    model=\"{model_name}\", # model to send to the proxy\n    messages = [\n        {{\n            \"role\": \"user\",\n            \"content\": \"this is a test request, write a short poem\"\n        }}\n    ]\n)\n```\n\nModel Info: \n```\n{model_info}\n```\n"
    )
}

fn sent_message(sender: &FakeSender) -> String {
    let sent = sender.alerts.entries();
    let alert = sent.first().expect("one model added alert");
    alert.message.clone()
}

#[test]
fn renders_the_exact_template_without_a_base_model() {
    let catalog = catalog(vec![(
        "litellm_name",
        object(vec![("max_tokens", integer(128))]),
    )]);

    let message = render_model_added_message(
        "my-model",
        "litellm_name",
        None,
        &catalog,
        Some("http://proxy.test:4321"),
    )
    .expect("the message renders");

    assert_eq!(
        message,
        expected_message(
            "my-model",
            "",
            "max_tokens: 128\n",
            "http://proxy.test:4321"
        )
    );
}

#[test]
fn a_base_model_adds_its_line_and_selects_the_base_entry() {
    let catalog = catalog(vec![
        ("litellm_name", object(vec![("max_tokens", integer(1))])),
        ("other/base", object(vec![("max_tokens", integer(2))])),
    ]);

    let message = render_model_added_message(
        "my-model",
        "litellm_name",
        Some("other/base"),
        &catalog,
        None,
    )
    .expect("the message renders");

    assert_eq!(
        message,
        expected_message(
            "my-model",
            "Base Model: `other/base`\n",
            "max_tokens: 2\n",
            "http://0.0.0.0:4000"
        )
    );
}

#[test]
fn an_empty_base_model_still_takes_the_base_branch() {
    let catalog = catalog(vec![
        ("litellm_name", object(vec![("max_tokens", integer(1))])),
        ("", object(vec![("max_tokens", integer(3))])),
    ]);

    let message = render_model_added_message("my-model", "litellm_name", Some(""), &catalog, None)
        .expect("the message renders");

    assert_eq!(
        message,
        expected_message(
            "my-model",
            "Base Model: ``\n",
            "max_tokens: 3\n",
            "http://0.0.0.0:4000"
        )
    );
}

#[test]
fn an_unknown_catalog_name_renders_an_empty_model_info() {
    let catalog = catalog(vec![]);

    let message = render_model_added_message("my-model", "missing", None, &catalog, None)
        .expect("the message renders");

    assert_eq!(
        message,
        expected_message("my-model", "", "", "http://0.0.0.0:4000")
    );
}

#[rstest]
#[case::micro_float(1.63e-06, "0.00000163")]
#[case::small_float(0.000002, "0.00000200")]
#[case::quarter_micro(2.5e-07, "0.00000025")]
#[case::large_float(1234.5, "1234.50000000")]
fn renders_costs_with_eight_decimals(#[case] cost: f64, #[case] expected: &str) {
    let catalog = catalog(vec![(
        "litellm_name",
        object(vec![("input_cost_per_token", float(cost))]),
    )]);

    let message = render_model_added_message("my-model", "litellm_name", None, &catalog, None)
        .expect("the message renders");

    assert!(message.contains(&format!("input_cost_per_token: {expected}\n")));
}

#[rstest]
#[case::integer_cost(Json::Number(Number::from(7_u32)), "7.00000000")]
#[case::true_cost(Json::Bool(true), "1.00000000")]
#[case::false_cost(Json::Bool(false), "0.00000000")]
fn renders_integer_and_bool_costs_with_eight_decimals(#[case] cost: Json, #[case] expected: &str) {
    let catalog = catalog(vec![(
        "litellm_name",
        object(vec![("output_cost_per_token", cost)]),
    )]);

    let message = render_model_added_message("my-model", "litellm_name", None, &catalog, None)
        .expect("the message renders");

    assert!(message.contains(&format!("output_cost_per_token: {expected}\n")));
}

#[rstest]
#[case::true_value(Json::Bool(true), "True")]
#[case::false_value(Json::Bool(false), "False")]
#[case::null_value(Json::Null, "None")]
#[case::integer_value(Json::Number(Number::from(42_u32)), "42")]
#[case::float_value(Json::Number(Number::from_f64(1.5).expect("a finite test number")), "1.5")]
#[case::text_value(Json::Text("openai".to_owned()), "openai")]
fn renders_scalars_in_python_text_form(#[case] value: Json, #[case] expected: &str) {
    let catalog = catalog(vec![("litellm_name", object(vec![("field", value)]))]);

    let message = render_model_added_message("my-model", "litellm_name", None, &catalog, None)
        .expect("the message renders");

    assert!(message.contains(&format!("field: {expected}\n")));
}

#[rstest]
#[case::list_value(Json::List(vec![Json::Text("us".to_owned()), Json::Text("eu".to_owned())]), "[\"us\",\"eu\"]")]
#[case::object_value(
    Json::Object(object(vec![("b", Json::Bool(true)), ("a", integer(1))])),
    "{\"b\":true,\"a\":1}"
)]
fn renders_nested_values_as_compact_json(#[case] value: Json, #[case] expected: &str) {
    let catalog = catalog(vec![("litellm_name", object(vec![("field", value)]))]);

    let message = render_model_added_message("my-model", "litellm_name", None, &catalog, None)
        .expect("the message renders");

    assert!(message.contains(&format!("field: {expected}\n")));
}

#[rstest]
#[case::text_cost(Json::Text("free".to_owned()))]
#[case::null_cost(Json::Null)]
#[case::list_cost(Json::List(vec![Json::Bool(true)]))]
fn a_non_number_cost_is_a_typed_error(#[case] cost: Json) {
    let catalog = catalog(vec![(
        "litellm_name",
        object(vec![("input_cost_per_token", cost)]),
    )]);

    let error = render_model_added_message("my-model", "litellm_name", None, &catalog, None)
        .expect_err("the cost does not render");

    assert!(matches!(
        error,
        ModelInfoRenderError::Cost { ref field } if field == "input_cost_per_token"
    ));
}

#[rstest]
#[case::unset(None, "base_url=http://0.0.0.0:4000\n")]
#[case::empty(Some(""), "base_url=\n")]
#[case::configured(Some("https://proxy.example"), "base_url=https://proxy.example\n")]
#[tokio::test]
async fn the_base_url_line_comes_from_the_environment(
    #[case] base_url: Option<&str>,
    #[case] expected: &str,
) {
    let sender = FakeSender::default();
    let service = alerts(catalog(vec![]), sender.clone(), environment(base_url));

    service
        .model_added_alert("my-model", "litellm_name", None)
        .await
        .expect("the alert is sent");

    assert!(sent_message(&sender).contains(expected));
}

#[tokio::test]
async fn the_alert_goes_out_low_as_new_model_added() {
    let sender = FakeSender::default();
    let service = alerts(catalog(vec![]), sender.clone(), environment(None));

    service
        .model_added_alert("my-model", "litellm_name", None)
        .await
        .expect("the alert is sent");

    let sent = sender.alerts.entries();
    let alert = sent.first().expect("one model added alert");
    assert_eq!(alert.level, AlertLevel::Low);
    assert_eq!(alert.alert_type, AlertType::NewModelAdded);
    assert!(alert.user_info.is_none());
    assert!(alert.metadata.is_empty());
    assert!(alert.kwargs.is_empty());
}

#[tokio::test]
async fn a_typed_render_error_sends_nothing() {
    let sender = FakeSender::default();
    let service = alerts(
        catalog(vec![(
            "litellm_name",
            object(vec![(
                "input_cost_per_token",
                Json::Text("free".to_owned()),
            )]),
        )]),
        sender.clone(),
        environment(None),
    );

    let result = service
        .model_added_alert("my-model", "litellm_name", None)
        .await;

    assert!(matches!(
        result,
        Err(ModelAddedAlertError::Render(
            ModelInfoRenderError::Cost { .. }
        ))
    ));
    assert!(sender.alerts.entries().is_empty());
}

#[tokio::test]
async fn a_sender_error_is_returned() {
    let service = alerts(catalog(vec![]), FakeSender::failing(), environment(None));

    let result = service
        .model_added_alert("my-model", "litellm_name", None)
        .await;

    assert!(matches!(result, Err(ModelAddedAlertError::Alert(_))));
}
