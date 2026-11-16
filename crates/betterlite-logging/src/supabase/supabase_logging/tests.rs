#![cfg(test)]

mod events;

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::rows::{InitiatedRow, RequestRow};
use super::settings::{SupabaseSettings, SupabaseSettingsError};
use super::{DEFAULT_TABLE_NAME, SupabaseLogError, SupabaseLogger};
use crate::http_analytics_loggers::delivery::{Delivery, DeliveryError, PostRequest, PostResponse};
use crate::supabase::client::SupabaseClient;

const BASE: &str = "https://project.supabase.test";
const MESSAGES: &str = r#"[{"role": "user", "content": "Hello"}]"#;
const RESPONSE: &str = r#"{"id": "chatcmpl-1", "model": "gpt-4o-2024-08-06", "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}}]}"#;

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture JSON decodes")
}

fn text(value: &Json) -> &str {
    let Json::Text(shown) = value else {
        panic!("the value is a JSON string");
    };
    shown
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Recorded {
    url: String,
    headers: BTreeMap<String, String>,
    body: Json,
}

impl Recorded {
    fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .map_or_else(|| panic!("the request carries {name}"), String::as_str)
    }

    fn field(&self, name: &str) -> &Json {
        let Json::Object(fields) = &self.body else {
            panic!("the body is a JSON object");
        };
        fields
            .get(name)
            .unwrap_or_else(|| panic!("the body carries {name}"))
    }
}

#[derive(Clone)]
struct Captured {
    status: u16,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl Captured {
    fn answering(status: u16) -> Self {
        Self {
            status,
            requests: Arc::default(),
        }
    }

    fn requests(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn only_request(&self) -> Recorded {
        let requests = self.requests();
        assert_eq!(requests.len(), 1, "exactly one request is written");
        requests.into_iter().next().expect("one request")
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the capture delivery failed")]
struct CaptureError;

impl Delivery for Captured {
    type Error = CaptureError;

    fn post(
        &self,
        request: PostRequest,
    ) -> impl Future<Output = Result<PostResponse, Self::Error>> + Send {
        let Ok(body) = serde_json::from_slice::<Json>(&request.body) else {
            return ready(Err(CaptureError));
        };
        let headers = request
            .headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|shown| (name.as_str().to_owned(), shown.to_owned()))
            })
            .collect();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: request.url.to_string(),
                headers,
                body,
            });
        ready(Ok(PostResponse {
            status: self.status,
            body: Vec::new(),
        }))
    }
}

fn logger_on(base: &str, delivery: Captured, table: &str) -> SupabaseLogger<Captured> {
    SupabaseLogger {
        client: SupabaseClient {
            base_url: Url::parse(base).expect("the fixture URL parses"),
            key: SecretString::from("service-key"),
            delivery,
        },
        table_name: table.to_owned(),
    }
}

fn fixture(table: &str) -> (SupabaseLogger<Captured>, Captured) {
    let capture = Captured::answering(201);
    (logger_on(BASE, capture.clone(), table), capture)
}

fn initiated_row() -> InitiatedRow {
    InitiatedRow {
        model: "gpt-4o".to_owned(),
        messages: json(MESSAGES),
        end_user: Json::Text("end-user-1".to_owned()),
        status: "initiated",
        litellm_call_id: "call-1".to_owned(),
    }
}

struct RowFixture {
    messages: Json,
    response: Json,
    start_time: Decimal,
    end_time: Decimal,
    total_cost: Decimal,
}

fn row_fixture() -> RowFixture {
    RowFixture {
        messages: json(MESSAGES),
        response: json(RESPONSE),
        start_time: Decimal::new(17_040_672_000, 1),
        end_time: Decimal::new(17_040_672_015, 1),
        total_cost: Decimal::new(125, 7),
    }
}

impl RowFixture {
    fn row(&self) -> RequestRow<'_> {
        RequestRow {
            messages: Some(&self.messages),
            end_user: Some("end-user-1"),
            response_obj: &self.response,
            start_time: self.start_time,
            end_time: self.end_time,
            litellm_call_id: "call-1",
            total_cost: self.total_cost,
        }
    }
}

#[test]
fn the_environment_supplies_the_client_credentials() {
    let environment = BTreeMap::from([
        ("SUPABASE_URL".to_owned(), BASE.to_owned()),
        ("SUPABASE_KEY".to_owned(), "service-key".to_owned()),
    ]);
    let settings = SupabaseSettings::from_environment(&environment).expect("the settings resolve");
    assert_eq!(settings.url.as_str(), "https://project.supabase.test/");
    assert_eq!(settings.key.expose_secret(), "service-key");
    assert_eq!(settings.table_name, DEFAULT_TABLE_NAME);
    assert_eq!(settings.table_name, "request_logs");
}

#[test]
fn a_missing_url_gives_lite_llms_message() {
    let environment = BTreeMap::from([("SUPABASE_KEY".to_owned(), "service-key".to_owned())]);
    let error = SupabaseSettings::from_environment(&environment)
        .err()
        .expect("the url is required");
    assert_eq!(
        error.to_string(),
        "LiteLLM Error, trying to use Supabase but url or key not passed. Create a table and set `litellm.supabase_url=<your-url>` and `litellm.supabase_key=<your-key>`"
    );
}

#[test]
fn a_missing_key_gives_lite_llms_message() {
    let environment = BTreeMap::from([("SUPABASE_URL".to_owned(), BASE.to_owned())]);
    let error = SupabaseSettings::from_environment(&environment)
        .err()
        .expect("the key is required");
    assert_eq!(
        error.to_string(),
        "LiteLLM Error, trying to use Supabase but url or key not passed. Create a table and set `litellm.supabase_url=<your-url>` and `litellm.supabase_key=<your-key>`"
    );
}

#[test]
fn an_empty_credential_counts_as_unset() {
    let empty_url = BTreeMap::from([
        ("SUPABASE_URL".to_owned(), String::new()),
        ("SUPABASE_KEY".to_owned(), "service-key".to_owned()),
    ]);
    assert!(matches!(
        SupabaseSettings::from_environment(&empty_url),
        Err(SupabaseSettingsError::MissingCredentials)
    ));
    let empty_key = BTreeMap::from([
        ("SUPABASE_URL".to_owned(), BASE.to_owned()),
        ("SUPABASE_KEY".to_owned(), String::new()),
    ]);
    assert!(matches!(
        SupabaseSettings::from_environment(&empty_key),
        Err(SupabaseSettingsError::MissingCredentials)
    ));
}

#[test]
fn a_url_that_is_not_http_is_refused() {
    let environment = BTreeMap::from([
        (
            "SUPABASE_URL".to_owned(),
            "ftp://project.supabase.test".to_owned(),
        ),
        ("SUPABASE_KEY".to_owned(), "service-key".to_owned()),
    ]);
    let error = SupabaseSettings::from_environment(&environment)
        .err()
        .expect("ftp is refused");
    assert_eq!(
        error.to_string(),
        "SUPABASE_URL must use the http or https scheme"
    );
}

#[test]
fn a_url_that_does_not_parse_is_refused() {
    let environment = BTreeMap::from([
        ("SUPABASE_URL".to_owned(), "not a url".to_owned()),
        ("SUPABASE_KEY".to_owned(), "service-key".to_owned()),
    ]);
    let error = SupabaseSettings::from_environment(&environment)
        .err()
        .expect("the URL is invalid");
    assert_eq!(error.to_string(), "SUPABASE_URL is not a valid URL");
}

#[tokio::test]
async fn an_initiated_row_inserts_the_call() {
    let (logger, captured) = fixture(DEFAULT_TABLE_NAME);
    logger
        .input_log_event(&initiated_row())
        .await
        .expect("the insert is accepted");
    let recorded = captured.only_request();
    assert_eq!(
        recorded.url,
        "https://project.supabase.test/rest/v1/request_logs"
    );
    assert_eq!(recorded.header("apikey"), "service-key");
    assert_eq!(recorded.header("authorization"), "Bearer service-key");
    assert_eq!(recorded.header("accept"), "application/json");
    assert_eq!(recorded.header("content-type"), "application/json");
    assert_eq!(recorded.header("prefer"), "return=representation");
    assert_eq!(
        recorded.body,
        json(
            r#"{
                "model": "gpt-4o",
                "messages": [{"role": "user", "content": "Hello"}],
                "end_user": "end-user-1",
                "status": "initiated",
                "litellm_call_id": "call-1"
            }"#
        )
    );
}

#[tokio::test]
async fn a_base_path_is_kept_on_the_table_route() {
    let capture = Captured::answering(201);
    let logger = logger_on(
        "https://project.supabase.test/gateway/",
        capture.clone(),
        "logs",
    );
    logger
        .input_log_event(&initiated_row())
        .await
        .expect("the insert is accepted");
    assert_eq!(
        capture.only_request().url,
        "https://project.supabase.test/gateway/rest/v1/logs"
    );
}

#[tokio::test]
async fn a_configured_table_name_replaces_the_default() {
    let (logger, captured) = fixture("custom_logs");
    logger
        .input_log_event(&initiated_row())
        .await
        .expect("the insert is accepted");
    assert_eq!(
        captured.only_request().url,
        "https://project.supabase.test/rest/v1/custom_logs"
    );
}

#[tokio::test]
async fn a_rejected_insert_is_an_error() {
    let rejected = Captured::answering(500);
    let logger = logger_on(BASE, rejected, DEFAULT_TABLE_NAME);
    let error = logger
        .input_log_event(&initiated_row())
        .await
        .expect_err("the rejection surfaces");
    assert!(matches!(
        error,
        SupabaseLogError::Delivery(DeliveryError::Status { status: 500 })
    ));
}

#[tokio::test]
async fn a_status_below_the_success_range_is_an_error() {
    let below = Captured::answering(199);
    let logger = logger_on(BASE, below, DEFAULT_TABLE_NAME);
    let error = logger
        .input_log_event(&initiated_row())
        .await
        .expect_err("the status surfaces");
    assert!(matches!(
        error,
        SupabaseLogError::Delivery(DeliveryError::Status { status: 199 })
    ));
}

#[tokio::test]
async fn a_success_row_upserts_the_response() {
    let (logger, captured) = fixture(DEFAULT_TABLE_NAME);
    let rows = row_fixture();
    logger
        .log_event(&rows.row())
        .await
        .expect("the upsert is accepted");
    let recorded = captured.only_request();
    assert_eq!(
        recorded.url,
        "https://project.supabase.test/rest/v1/request_logs?on_conflict=litellm_call_id"
    );
    assert_eq!(
        recorded.header("prefer"),
        "resolution=merge-duplicates,return=representation"
    );
    assert_eq!(
        recorded.body,
        json(
            r#"{
                "response_time": 1.5,
                "model": "gpt-4o-2024-08-06",
                "total_cost": 0.0000125,
                "messages": [{"role": "user", "content": "Hello"}],
                "response": "hi",
                "end_user": "end-user-1",
                "litellm_call_id": "call-1",
                "status": "success"
            }"#
        )
    );
}

#[tokio::test]
async fn a_success_row_ignores_an_error_key_beside_the_choices() {
    let (logger, captured) = fixture(DEFAULT_TABLE_NAME);
    let mut rows = row_fixture();
    rows.response = json(
        r#"{"model": "gpt-4o-2024-08-06", "error": "Unable to map your input to a model.", "choices": [{"message": {"content": "hi"}}]}"#,
    );
    logger
        .log_event(&rows.row())
        .await
        .expect("the upsert is accepted");
    let recorded = captured.only_request();
    assert_eq!(text(recorded.field("status")), "success");
    assert_eq!(recorded.field("total_cost"), &json("0.0000125"));
}

#[tokio::test]
async fn a_success_row_without_a_response_model_is_not_posted() {
    let (logger, captured) = fixture(DEFAULT_TABLE_NAME);
    let mut rows = row_fixture();
    rows.response = json(r#"{"choices": [{"message": {"content": "hi"}}]}"#);
    logger
        .log_event(&rows.row())
        .await
        .expect("the event is dropped");
    assert!(captured.requests().is_empty());
}

#[tokio::test]
async fn a_success_row_without_choices_is_not_posted() {
    let (logger, captured) = fixture(DEFAULT_TABLE_NAME);
    let mut rows = row_fixture();
    rows.response = json(r#"{"model": "gpt-4o-2024-08-06"}"#);
    logger
        .log_event(&rows.row())
        .await
        .expect("the event is dropped");
    assert!(captured.requests().is_empty());
}

#[tokio::test]
async fn a_success_row_with_an_empty_choice_list_is_not_posted() {
    let (logger, captured) = fixture(DEFAULT_TABLE_NAME);
    let mut rows = row_fixture();
    rows.response = json(r#"{"model": "gpt-4o-2024-08-06", "choices": []}"#);
    logger
        .log_event(&rows.row())
        .await
        .expect("the event is dropped");
    assert!(captured.requests().is_empty());
}

#[tokio::test]
async fn a_success_row_with_non_list_choices_is_not_posted() {
    let (logger, captured) = fixture(DEFAULT_TABLE_NAME);
    let mut rows = row_fixture();
    rows.response = json(r#"{"model": "gpt-4o-2024-08-06", "choices": "hi"}"#);
    logger
        .log_event(&rows.row())
        .await
        .expect("the event is dropped");
    assert!(captured.requests().is_empty());
}

#[tokio::test]
async fn a_success_row_without_message_content_is_not_posted() {
    let (logger, captured) = fixture(DEFAULT_TABLE_NAME);
    let mut rows = row_fixture();
    rows.response =
        json(r#"{"model": "gpt-4o-2024-08-06", "choices": [{"message": {"role": "assistant"}}]}"#);
    logger
        .log_event(&rows.row())
        .await
        .expect("the event is dropped");
    assert!(captured.requests().is_empty());
}

#[tokio::test]
async fn a_response_that_is_not_an_object_is_not_posted() {
    let (logger, captured) = fixture(DEFAULT_TABLE_NAME);
    let mut rows = row_fixture();
    rows.response = Json::Text("hi".to_owned());
    logger
        .log_event(&rows.row())
        .await
        .expect("the event is dropped");
    assert!(captured.requests().is_empty());
}

#[tokio::test]
async fn a_failure_row_keeps_the_error_and_cost() {
    let (logger, captured) = fixture(DEFAULT_TABLE_NAME);
    let mut rows = row_fixture();
    rows.response = json(r#"{"model": "gpt-4o", "error": "upstream returned 500"}"#);
    logger
        .log_event(&rows.row())
        .await
        .expect("the upsert is accepted");
    let recorded = captured.only_request();
    assert_eq!(
        recorded.url,
        "https://project.supabase.test/rest/v1/request_logs?on_conflict=litellm_call_id"
    );
    assert_eq!(
        recorded.body,
        json(
            r#"{
                "response_time": 1.5,
                "model": "gpt-4o",
                "total_cost": 0.0000125,
                "messages": [{"role": "user", "content": "Hello"}],
                "error": "upstream returned 500",
                "end_user": "end-user-1",
                "litellm_call_id": "call-1",
                "status": "failure"
            }"#
        )
    );
}

#[tokio::test]
async fn an_unknown_model_error_zeroes_the_cost() {
    let (logger, captured) = fixture(DEFAULT_TABLE_NAME);
    let mut rows = row_fixture();
    rows.response = json(r#"{"model": "gpt-4o", "error": "Unable to map your input to a model."}"#);
    logger
        .log_event(&rows.row())
        .await
        .expect("the upsert is accepted");
    assert_eq!(captured.only_request().field("total_cost"), &json("0"));
}

#[tokio::test]
async fn an_unknown_model_error_inside_a_longer_text_zeroes_the_cost() {
    let (logger, captured) = fixture(DEFAULT_TABLE_NAME);
    let mut rows = row_fixture();
    rows.response = json(
        r#"{"model": "gpt-4o", "error": "litellm.BadRequestError: Unable to map your input to a model. Received Model Group=unknown"}"#,
    );
    logger
        .log_event(&rows.row())
        .await
        .expect("the upsert is accepted");
    assert_eq!(captured.only_request().field("total_cost"), &json("0"));
}

#[tokio::test]
async fn a_near_miss_unknown_model_error_keeps_the_cost() {
    let (logger, captured) = fixture(DEFAULT_TABLE_NAME);
    let mut rows = row_fixture();
    rows.response = json(r#"{"model": "gpt-4o", "error": "Unable to map your input to a model"}"#);
    logger
        .log_event(&rows.row())
        .await
        .expect("the upsert is accepted");
    assert_eq!(
        captured.only_request().field("total_cost"),
        &json("0.0000125")
    );
}

#[tokio::test]
async fn a_non_text_error_is_not_posted() {
    let (logger, captured) = fixture(DEFAULT_TABLE_NAME);
    let mut rows = row_fixture();
    rows.response = json(r#"{"model": "gpt-4o", "error": 500}"#);
    logger
        .log_event(&rows.row())
        .await
        .expect("the event is dropped");
    assert!(captured.requests().is_empty());
}

#[tokio::test]
async fn a_request_span_that_overflows_is_an_error() {
    let (logger, _captured) = fixture(DEFAULT_TABLE_NAME);
    let mut rows = row_fixture();
    rows.start_time = Decimal::MAX;
    rows.end_time = Decimal::MIN;
    let error = logger
        .log_event(&rows.row())
        .await
        .expect_err("the overflow surfaces");
    assert!(matches!(error, SupabaseLogError::ResponseTime));
}
