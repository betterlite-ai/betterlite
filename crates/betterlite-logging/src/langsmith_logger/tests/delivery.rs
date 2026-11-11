use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{
    FakeDelivery, Options, dynamic, event, harness, object_field, text, text_field,
};
use crate::callback_logger::CallbackLogger as _;
use crate::langsmith_logger::delivery::endpoint_url;

#[rstest]
#[case::default_base(
    "https://api.smith.langchain.com",
    "https://api.smith.langchain.com/api/v1/runs/batch"
)]
#[case::trailing_slash(
    "https://api.smith.langchain.com/",
    "https://api.smith.langchain.com/api/v1/runs/batch"
)]
#[case::version_present(
    "https://langsmith.example/api/v1",
    "https://langsmith.example/api/v1/runs/batch"
)]
#[case::version_with_slash(
    "https://langsmith.example/api/v1/",
    "https://langsmith.example/api/v1/runs/batch"
)]
#[case::custom_path(
    "https://langsmith.example/custom",
    "https://langsmith.example/custom/api/v1/runs/batch"
)]
#[case::empty_base("", "/api/v1/runs/batch")]
fn endpoint_url_tests_behavior(#[case] base: &str, #[case] expected: &str) {
    assert_eq!(endpoint_url(base), expected);
}

fn global_key() -> [(&'static str, &'static str); 1] {
    [("LANGSMITH_API_KEY", "global-key")]
}

fn header<'headers>(headers: &'headers http::HeaderMap, name: &str) -> Option<&'headers str> {
    let value = headers.get(name)?;
    value.to_str().ok()
}

#[tokio::test]
async fn runs_with_one_credential_set_post_once() {
    let options = Options {
        environment: &global_key(),
        batch_size: Some(2),
        draws: &[0.0_f64, 0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    built
        .logger
        .async_log_success_event(Arc::new(event(dynamic(&[]))))
        .await
        .expect("the event is logged");
    assert!(built.delivery.posts().is_empty());
    built
        .logger
        .async_log_success_event(Arc::new(event(dynamic(&[]))))
        .await
        .expect("the event is logged");
    let posts = built.delivery.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].runs().len(), 2);
    assert_eq!(
        posts[0].url,
        "https://api.smith.langchain.com/api/v1/runs/batch"
    );
    assert_eq!(header(&posts[0].headers, "x-api-key"), Some("global-key"));
    assert_eq!(
        header(&posts[0].headers, "content-type"),
        Some("application/json")
    );
}

#[tokio::test]
async fn two_credential_sets_post_one_batch_each() {
    let options = Options {
        environment: &[],
        batch_size: Some(2),
        draws: &[0.0_f64, 0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    let first = event(dynamic(&[
        ("langsmith_api_key", text("key-a")),
        ("langsmith_project", text("project-a")),
        ("langsmith_base_url", text("https://a.example")),
    ]));
    let second = event(dynamic(&[
        ("langsmith_api_key", text("key-b")),
        ("langsmith_project", text("project-b")),
        ("langsmith_base_url", text("https://b.example")),
        ("langsmith_tenant_id", text("tenant-b")),
    ]));
    built
        .logger
        .async_log_success_event(Arc::new(first))
        .await
        .expect("the event is logged");
    built
        .logger
        .async_log_success_event(Arc::new(second))
        .await
        .expect("the event is logged");
    let posts = built.delivery.posts();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0].url, "https://a.example/api/v1/runs/batch");
    assert_eq!(posts[1].url, "https://b.example/api/v1/runs/batch");
    assert_eq!(header(&posts[0].headers, "x-api-key"), Some("key-a"));
    assert_eq!(header(&posts[1].headers, "x-api-key"), Some("key-b"));
    assert_eq!(header(&posts[0].headers, "x-tenant-id"), None);
    assert_eq!(header(&posts[1].headers, "x-tenant-id"), Some("tenant-b"));
    assert_eq!(posts[0].runs().len(), 1);
    assert_eq!(posts[1].runs().len(), 1);
}

#[tokio::test]
async fn two_tenants_share_a_key_but_post_separately() {
    let options = Options {
        environment: &[("LANGSMITH_API_KEY", "shared-key")],
        batch_size: Some(2),
        draws: &[0.0_f64, 0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    let first = event(dynamic(&[("langsmith_tenant_id", text("tenant-a"))]));
    let second = event(dynamic(&[("langsmith_tenant_id", text("tenant-b"))]));
    built
        .logger
        .async_log_success_event(Arc::new(first))
        .await
        .expect("the event is logged");
    built
        .logger
        .async_log_success_event(Arc::new(second))
        .await
        .expect("the event is logged");
    let posts = built.delivery.posts();
    assert_eq!(posts.len(), 2);
    assert_eq!(header(&posts[0].headers, "x-api-key"), Some("shared-key"));
    assert_eq!(header(&posts[0].headers, "x-tenant-id"), Some("tenant-a"));
    assert_eq!(header(&posts[1].headers, "x-tenant-id"), Some("tenant-b"));
}

#[tokio::test]
async fn two_base_urls_share_credentials_but_post_separately() {
    let options = Options {
        environment: &[],
        batch_size: Some(2),
        draws: &[0.0_f64, 0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    let first = event(dynamic(&[
        ("langsmith_api_key", text("shared-key")),
        ("langsmith_base_url", text("https://a.example")),
    ]));
    let second = event(dynamic(&[
        ("langsmith_api_key", text("shared-key")),
        ("langsmith_base_url", text("https://b.example")),
    ]));
    built
        .logger
        .async_log_success_event(Arc::new(first))
        .await
        .expect("the event is logged");
    built
        .logger
        .async_log_success_event(Arc::new(second))
        .await
        .expect("the event is logged");
    let posts = built.delivery.posts();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0].url, "https://a.example/api/v1/runs/batch");
    assert_eq!(posts[1].url, "https://b.example/api/v1/runs/batch");
    assert_eq!(posts[0].runs().len(), 1);
    assert_eq!(posts[1].runs().len(), 1);
}

#[tokio::test]
async fn two_projects_share_credentials_but_post_separately() {
    let options = Options {
        environment: &[("LANGSMITH_API_KEY", "shared-key")],
        batch_size: Some(2),
        draws: &[0.0_f64, 0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    let first = event(dynamic(&[("langsmith_project", text("project-a"))]));
    let second = event(dynamic(&[("langsmith_project", text("project-b"))]));
    built
        .logger
        .async_log_success_event(Arc::new(first))
        .await
        .expect("the event is logged");
    built
        .logger
        .async_log_success_event(Arc::new(second))
        .await
        .expect("the event is logged");
    let posts = built.delivery.posts();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0].runs().len(), 1);
    assert_eq!(posts[1].runs().len(), 1);
    assert_eq!(
        text_field(&posts[0].runs()[0], "session_name"),
        Some("project-a")
    );
    assert_eq!(
        text_field(&posts[1].runs()[0], "session_name"),
        Some("project-b")
    );
}

#[tokio::test]
async fn a_run_without_an_api_key_is_skipped_with_a_warning() {
    let options = Options {
        environment: &[],
        batch_size: Some(1),
        draws: &[0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    let (captured, guard) = super::support::capture_logs();
    built
        .logger
        .async_log_success_event(Arc::new(event(dynamic(&[
            ("langsmith_project", text("project-a")),
            ("langsmith_base_url", text("https://a.example")),
        ]))))
        .await
        .expect("a skipped run is not an error");
    drop(guard);
    assert!(built.delivery.posts().is_empty());
    assert!(captured.contents().contains("credentials missing"));
}

#[tokio::test]
async fn a_status_of_300_or_more_is_logged_as_an_error() {
    let options = Options {
        environment: &global_key(),
        batch_size: Some(1),
        draws: &[0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(500));
    let (captured, guard) = super::support::capture_logs();
    built
        .logger
        .async_log_success_event(Arc::new(event(dynamic(&[]))))
        .await
        .expect("the endpoint status is not returned");
    drop(guard);
    let log = captured.contents();
    assert!(log.contains("Langsmith HTTP Error"));
    assert!(log.contains("500"));
    assert!(log.contains("the endpoint answered"));
}

#[tokio::test]
async fn a_successful_post_is_logged_at_debug() {
    let options = Options {
        environment: &global_key(),
        batch_size: Some(1),
        draws: &[0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    let (captured, guard) = super::support::capture_logs();
    built
        .logger
        .async_log_success_event(Arc::new(event(dynamic(&[]))))
        .await
        .expect("the event is logged");
    drop(guard);
    let log = captured.contents();
    assert!(log.contains("Batch of runs successfully created"));
    assert!(!log.contains("Langsmith HTTP Error"));
}

#[tokio::test]
async fn a_transport_error_is_logged_and_swallowed() {
    let options = Options {
        environment: &global_key(),
        batch_size: Some(1),
        draws: &[0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::failing());
    let (captured, guard) = super::support::capture_logs();
    built
        .logger
        .async_log_failure_event(Arc::new(event(dynamic(&[]))))
        .await
        .expect("a transport error is not returned");
    drop(guard);
    assert!(
        captured
            .contents()
            .contains("Langsmith Layer Error - the request failed")
    );
    assert_eq!(built.delivery.posts().len(), 1);
}

#[rstest]
#[case::default(&[], "LLMRun")]
#[case::configured(&[("LANGSMITH_DEFAULT_RUN_NAME", "my-run")], "my-run")]
#[tokio::test]
async fn the_run_name_reaches_the_batch_body(
    #[case] extra: &[(&str, &str)],
    #[case] expected: &str,
) {
    let mut environment = vec![("LANGSMITH_API_KEY", "global-key")];
    environment.extend_from_slice(extra);
    let options = Options {
        environment: &environment,
        batch_size: Some(1),
        draws: &[0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    built
        .logger
        .async_log_success_event(Arc::new(event(dynamic(&[]))))
        .await
        .expect("the event is logged");
    let posts = built.delivery.posts();
    assert_eq!(posts.len(), 1);
    let runs = posts[0].runs();
    assert_eq!(text_field(&runs[0], "name"), Some(expected));
}

#[tokio::test]
async fn request_metadata_reaches_the_run_name_and_session_name() {
    let options = Options {
        environment: &global_key(),
        batch_size: Some(1),
        draws: &[0.0_f64],
        redact: false,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    let metadata: IndexMap<String, Json> = IndexMap::from([
        ("run_name".to_owned(), text("nightly-eval")),
        ("project_name".to_owned(), text("evals")),
    ]);
    let mut request = event(dynamic(&[]));
    request.litellm_metadata = Arc::new(metadata);
    built
        .logger
        .async_log_success_event(Arc::new(request))
        .await
        .expect("the event is logged");
    let posts = built.delivery.posts();
    assert_eq!(posts.len(), 1);
    let runs = posts[0].runs();
    assert_eq!(text_field(&runs[0], "name"), Some("nightly-eval"));
    assert_eq!(text_field(&runs[0], "session_name"), Some("evals"));
}

#[rstest]
#[case::redacted(true, None)]
#[case::kept(false, Some("secret"))]
#[tokio::test]
async fn the_redaction_flag_controls_the_requester_metadata(
    #[case] redact: bool,
    #[case] expected: Option<&str>,
) {
    let options = Options {
        environment: &global_key(),
        batch_size: Some(1),
        draws: &[0.0_f64],
        redact,
    };
    let built = harness(&options, FakeDelivery::answering(200));
    built
        .logger
        .async_log_success_event(Arc::new(event(dynamic(&[]))))
        .await
        .expect("the event is logged");
    let posts = built.delivery.posts();
    assert_eq!(posts.len(), 1);
    let runs = posts[0].runs();
    let inputs = object_field(&runs[0], "inputs").expect("the run carries its inputs");
    let metadata = object_field(inputs, "metadata").expect("the inputs carry the metadata");
    let requester =
        object_field(metadata, "requester_metadata").expect("the metadata carries the requester");
    assert_eq!(text_field(requester, "user_api_key"), expected);
}
