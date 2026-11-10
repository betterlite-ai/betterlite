use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValueKind;
use opentelemetry_proto::tonic::resource::v1::Resource as ProtoResource;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{SpanData, SpanExporter as _};
use prost::Message as _;
use rstest::rstest;
use tokio::runtime::Builder;

use crate::langfuse_exporter::TRUNCATION_MARKER;
use crate::langfuse_exporter::discard::DiscardingSpanExporter;
use crate::langfuse_exporter::encoder::OtlpSpanEncoder;
use crate::langfuse_exporter::endpoint::endpoint;
use crate::langfuse_exporter::error::LangfuseExporterError;
use crate::langfuse_exporter::exporter::LangfuseSpanExporter;
use crate::langfuse_exporter::http_transport::HttpTransport;
use crate::langfuse_exporter::sleep::Sleep;

use super::server::{reply, scripted};
use super::support::{
    CLOUD_ENDPOINT, DecodedSpan, RecordingSleep, ScriptedTransport, Step, UNENCODABLE_KEY,
    attribute, captured, exporter_with, header, options_for, rejecting_exporter_with, span,
    spans_of,
};

const INPUT: &str = "langfuse.observation.input";
const OUTPUT: &str = "langfuse.observation.output";
const METADATA: &str = "langfuse.observation.metadata";

fn run<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn export(exporter: &LangfuseSpanExporter<ScriptedTransport>, batch: Vec<SpanData>) -> bool {
    run(exporter.export(batch)).is_ok()
}

fn names(batches: &[Vec<DecodedSpan>]) -> Vec<Vec<&str>> {
    batches
        .iter()
        .map(|batch| batch.iter().map(|span| span.name.as_str()).collect())
        .collect()
}

#[rstest]
#[case::trailing_base_and_leading_path(
    "https://langfuse.example/",
    "/custom/traces",
    "https://langfuse.example/custom/traces"
)]
#[case::neither_side_slashed(
    "https://langfuse.example",
    "custom/traces",
    "https://langfuse.example/custom/traces"
)]
#[case::many_slashes(
    "https://langfuse.example///",
    "///custom/traces",
    "https://langfuse.example/custom/traces"
)]
fn joins_the_base_url_and_the_export_path(
    #[case] base_url: &str,
    #[case] export_path: &str,
    #[case] expected: &str,
) {
    assert_eq!(endpoint(base_url, export_path), expected);
}

#[rstest]
#[case::unset(&[], CLOUD_ENDPOINT)]
#[case::empty_counts_as_unset(&[("LANGFUSE_OTEL_TRACES_EXPORT_PATH", "")], CLOUD_ENDPOINT)]
#[case::custom(
    &[("LANGFUSE_OTEL_TRACES_EXPORT_PATH", "/custom/otel/traces")],
    "https://cloud.langfuse.com/custom/otel/traces"
)]
fn requests_the_export_path_the_environment_sets_or_the_default(
    #[case] environment: &[(&str, &str)],
    #[case] expected: &str,
) {
    let fixture = exporter_with(vec![Step::Status(200)], environment);
    assert!(export(&fixture.exporter, vec![span("one", &[])]));
    assert_eq!(fixture.transport.seen()[0].url, expected);
}

#[test]
fn sends_the_basic_authorization_and_every_sdk_header() {
    let fixture = exporter_with(vec![Step::Status(200)], &[]);
    assert!(export(&fixture.exporter, vec![span("one", &[])]));
    let pairs = &fixture.transport.seen()[0].headers;
    let expected = format!("Basic {}", STANDARD.encode("pk-lf-test:sk-lf-test"));
    assert_eq!(pairs.len(), 6);
    let sent = [
        "authorization",
        "content-type",
        "x-langfuse-sdk-name",
        "x-langfuse-sdk-version",
        "x-langfuse-public-key",
        "x-langfuse-ingestion-version",
    ]
    .map(|name| header(pairs, name));
    let wanted = [
        expected.as_str(),
        "application/x-protobuf",
        "python",
        "4.15.2",
        "pk-lf-test",
        "4",
    ]
    .map(|value| Some(value.to_owned()));
    assert_eq!(sent, wanted);
}

#[test]
fn the_debug_output_names_the_headers_without_printing_a_value() {
    let fixture = exporter_with(Vec::new(), &[]);
    let debug = format!("{:?}", fixture.exporter);
    assert!(debug.contains("authorization"));
    assert!(!debug.contains("sk-lf-test"));
    assert!(!debug.contains("Basic "));
    assert!(!debug.contains("pk-lf-test"));
}

#[rstest]
#[case::request_timeout(408)]
#[case::rate_limit(429)]
#[case::server_error(500)]
#[case::service_unavailable(503)]
#[case::last_5xx(599)]
fn retries_the_statuses_litellm_retries(#[case] status: u16) {
    let fixture = exporter_with(vec![Step::Status(status), Step::Status(200)], &[]);
    assert!(export(&fixture.exporter, vec![span("one", &[])]));
    assert_eq!(fixture.transport.seen().len(), 2);
    assert_eq!(fixture.sleep.seconds(), vec![1]);
}

#[rstest]
#[case::bad_request(400)]
#[case::unauthorized(401)]
#[case::not_found(404)]
#[case::proxy_auth(407)]
#[case::conflict(409)]
fn rejects_the_statuses_litellm_does_not_retry(#[case] status: u16) {
    let fixture = exporter_with(vec![Step::Status(status)], &[]);
    assert!(!export(&fixture.exporter, vec![span("one", &[])]));
    assert_eq!(fixture.transport.seen().len(), 1);
    assert!(fixture.sleep.seconds().is_empty());
}

#[rstest]
#[case::ok(200)]
#[case::no_content(204)]
#[case::last_2xx(299)]
fn accepts_every_2xx_status(#[case] status: u16) {
    let fixture = exporter_with(vec![Step::Status(status)], &[]);
    assert!(export(&fixture.exporter, vec![span("one", &[])]));
    assert_eq!(fixture.transport.seen().len(), 1);
}

#[rstest]
#[case::informational(100)]
#[case::last_below_2xx(199)]
#[case::first_3xx(300)]
#[case::redirect(302)]
#[case::not_modified(304)]
fn rejects_the_statuses_httpx_raises_for_below_400(#[case] status: u16) {
    let fixture = exporter_with(vec![Step::Status(status)], &[]);
    assert!(!export(&fixture.exporter, vec![span("one", &[])]));
    assert_eq!(fixture.transport.seen().len(), 1);
    assert!(fixture.sleep.seconds().is_empty());
}

#[test]
fn retries_a_transport_failure() {
    let fixture = exporter_with(
        vec![Step::Transport, Step::Transport, Step::Status(200)],
        &[],
    );
    assert!(export(&fixture.exporter, vec![span("one", &[])]));
    assert_eq!(fixture.transport.seen().len(), 3);
    assert_eq!(fixture.sleep.seconds(), vec![1, 2]);
}

#[rstest]
#[case::no_retries(0, vec![])]
#[case::one_retry(1, vec![1])]
#[case::default(3, vec![1, 2, 4])]
#[case::reaches_the_ceiling(7, vec![1, 2, 4, 8, 16, 32, 64])]
#[case::stays_at_the_ceiling(8, vec![1, 2, 4, 8, 16, 32, 64, 64])]
fn backs_off_between_retries(#[case] retries: u32, #[case] expected: Vec<u64>) {
    let count = usize::try_from(retries).unwrap().saturating_add(1);
    let retries_text = retries.to_string();
    let fixture = exporter_with(
        vec![Step::Status(500); count],
        &[("LANGFUSE_MAX_RETRIES", retries_text.as_str())],
    );
    assert!(!export(&fixture.exporter, vec![span("one", &[])]));
    assert_eq!(fixture.sleep.seconds(), expected);
    assert_eq!(
        fixture.transport.seen().len(),
        expected.len().saturating_add(1)
    );
}

#[test]
fn the_retry_ceiling_of_a_thousand_keeps_the_backoff_at_sixty_four_seconds() {
    let fixture = exporter_with(
        vec![Step::Status(500); 1001],
        &[("LANGFUSE_MAX_RETRIES", "1000")],
    );
    assert!(!export(&fixture.exporter, vec![span("one", &[])]));
    let seconds = fixture.sleep.seconds();
    assert_eq!(seconds.len(), 1000);
    assert_eq!(&seconds[..7], &[1, 2, 4, 8, 16, 32, 64]);
    assert_eq!(seconds.last(), Some(&64));
}

#[test]
fn halves_a_batch_that_is_too_large() {
    let fixture = exporter_with(
        vec![Step::Status(413), Step::Status(200), Step::Status(200)],
        &[],
    );
    let batch = vec![span("a", &[]), span("b", &[]), span("c", &[])];
    assert!(export(&fixture.exporter, batch));
    assert_eq!(
        names(&spans_of(&fixture.transport.seen())),
        vec![vec!["a", "b", "c"], vec!["a"], vec!["b", "c"]]
    );
    assert!(fixture.sleep.seconds().is_empty());
}

#[test]
fn an_empty_batch_answered_with_413_settles_after_one_request() {
    let fixture = exporter_with(vec![Step::Status(413)], &[]);
    assert!(export(&fixture.exporter, Vec::new()));
    assert_eq!(fixture.transport.seen().len(), 1);
}

#[test]
fn a_rejected_half_keeps_the_export_failed_after_a_later_round_delivers() {
    let fixture = exporter_with(
        vec![
            Step::Status(413),
            Step::Status(400),
            Step::Status(413),
            Step::Status(200),
        ],
        &[],
    );
    let batch = vec![span("a", &[]), span("b", &[(INPUT, "hello")])];
    assert!(!export(&fixture.exporter, batch));
    let requests = spans_of(&fixture.transport.seen());
    assert_eq!(
        names(&requests),
        vec![vec!["a", "b"], vec!["a"], vec!["b"], vec!["b"]]
    );
    assert_eq!(attribute(&requests[2][0], INPUT), Some("hello"));
    assert_eq!(attribute(&requests[3][0], INPUT), Some(TRUNCATION_MARKER));
}

#[test]
fn truncates_the_largest_field_of_a_single_span() {
    let fixture = exporter_with(
        vec![
            Step::Status(413),
            Step::Status(413),
            Step::Status(413),
            Step::Status(200),
        ],
        &[],
    );
    let batch = vec![span(
        "one",
        &[(INPUT, "0123456789"), (OUTPUT, "ab"), (METADATA, "x")],
    )];
    assert!(export(&fixture.exporter, batch));
    let requests = spans_of(&fixture.transport.seen());
    assert_eq!(requests.len(), 4);
    assert_eq!(attribute(&requests[0][0], INPUT), Some("0123456789"));
    assert_eq!(attribute(&requests[1][0], INPUT), Some(TRUNCATION_MARKER));
    assert_eq!(attribute(&requests[1][0], OUTPUT), Some("ab"));
    assert_eq!(attribute(&requests[2][0], OUTPUT), Some(TRUNCATION_MARKER));
    assert_eq!(
        attribute(&requests[3][0], "langfuse.observation.metadata.truncated"),
        Some(TRUNCATION_MARKER)
    );
    assert_eq!(attribute(&requests[3][0], METADATA), None);
}

#[rstest]
#[case::output_is_largest("a", "bbbb", OUTPUT)]
#[case::characters_not_bytes_decide(
    "\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442} \u{43c}\u{438}\u{440}",
    "abcdefghijkl",
    OUTPUT
)]
#[case::a_tie_keeps_the_first_group("aa", "bb", INPUT)]
fn truncates_the_larger_group_first(
    #[case] input: &str,
    #[case] output: &str,
    #[case] marked: &str,
) {
    let fixture = exporter_with(vec![Step::Status(413), Step::Status(200)], &[]);
    let batch = vec![span("one", &[(INPUT, input), (OUTPUT, output)])];
    assert!(export(&fixture.exporter, batch));
    let requests = spans_of(&fixture.transport.seen());
    assert_eq!(attribute(&requests[1][0], marked), Some(TRUNCATION_MARKER));
    assert_eq!(attribute(&requests[1][0], "langfuse.trace.input"), None);
    assert_eq!(attribute(&requests[1][0], "langfuse.trace.output"), None);
}

#[test]
fn marks_every_prefix_of_the_truncated_metadata_group() {
    let fixture = exporter_with(vec![Step::Status(413), Step::Status(200)], &[]);
    let batch = vec![span(
        "one",
        &[
            ("langfuse.observation.metadata.model", "gpt"),
            ("langfuse.trace.metadata.team", "core"),
        ],
    )];
    assert!(export(&fixture.exporter, batch));
    let requests = spans_of(&fixture.transport.seen());
    assert_eq!(
        attribute(&requests[1][0], "langfuse.observation.metadata.truncated"),
        Some(TRUNCATION_MARKER)
    );
    assert_eq!(
        attribute(&requests[1][0], "langfuse.trace.metadata.truncated"),
        Some(TRUNCATION_MARKER)
    );
    assert_eq!(
        attribute(&requests[1][0], "langfuse.observation.metadata.model"),
        None
    );
    assert_eq!(
        attribute(&requests[1][0], "langfuse.trace.metadata.team"),
        None
    );
}

#[test]
fn drops_a_span_still_too_large_with_every_field_truncated() {
    let fixture = exporter_with(vec![Step::Status(413); 4], &[]);
    let batch = vec![span("one", &[(INPUT, "x"), (OUTPUT, "y"), (METADATA, "z")])];
    assert!(!export(&fixture.exporter, batch));
    let seen = fixture.transport.seen();
    assert_eq!(seen.len(), 4);
    let last = &spans_of(&seen)[3][0];
    assert_eq!(attribute(last, INPUT), Some(TRUNCATION_MARKER));
    assert_eq!(attribute(last, OUTPUT), Some(TRUNCATION_MARKER));
    assert_eq!(
        attribute(last, "langfuse.observation.metadata.truncated"),
        Some(TRUNCATION_MARKER)
    );
}

#[test]
fn drops_only_the_spans_the_encoder_rejects() {
    let fixture = rejecting_exporter_with(vec![Step::Status(200)]);
    let batch = vec![
        span("a", &[]),
        span("b", &[(UNENCODABLE_KEY, "true")]),
        span("c", &[]),
    ];
    assert!(export(&fixture.exporter, batch));
    let seen = fixture.transport.seen();
    assert_eq!(names(&spans_of(&seen)), vec![vec!["a", "c"]]);
}

#[test]
fn a_batch_whose_spans_are_all_rejected_is_not_sent() {
    let fixture = rejecting_exporter_with(Vec::new());
    let batch = vec![
        span("a", &[(UNENCODABLE_KEY, "true")]),
        span("b", &[(UNENCODABLE_KEY, "true")]),
    ];
    assert!(!export(&fixture.exporter, batch));
    assert!(fixture.transport.seen().is_empty());
}

#[test]
fn encodes_the_resource_the_provider_set() {
    let mut fixture = exporter_with(vec![Step::Status(200)], &[]);
    fixture.exporter.set_resource(
        &Resource::builder_empty()
            .with_service_name("langfuse-test")
            .build(),
    );
    assert!(export(&fixture.exporter, vec![span("one", &[])]));
    let body = &fixture.transport.seen()[0].body;
    let request = ExportTraceServiceRequest::decode(body.as_slice()).unwrap();
    let resource = request.resource_spans[0].resource.as_ref().unwrap();
    assert_eq!(
        attribute_resource(resource, "service.name").as_deref(),
        Some("langfuse-test")
    );
}

fn attribute_resource(resource: &ProtoResource, name: &str) -> Option<String> {
    let found = resource
        .attributes
        .iter()
        .find(|attribute| attribute.key == name);
    let value = found?.value.as_ref()?.value.as_ref()?;
    if let AnyValueKind::StringValue(text) = value {
        Some(text.clone())
    } else {
        None
    }
}

#[test]
fn the_discarding_exporter_accepts_spans_without_a_transport() {
    let exporter = DiscardingSpanExporter;
    run(exporter.export(vec![span("one", &[])])).unwrap();
    exporter.force_flush().unwrap();
    exporter.shutdown().unwrap();
}

#[rstest]
#[case::missing_route(404, true)]
#[case::bad_request(400, false)]
fn points_only_a_404_at_the_self_hosted_server_floor(#[case] status: u16, #[case] hinted: bool) {
    let fixture = exporter_with(vec![Step::Status(status)], &[]);
    let (delivered, logs) = captured(|| export(&fixture.exporter, vec![span("one", &[])]));
    assert!(!delivered);
    assert_eq!(logs.contains("3.63.0"), hinted);
}

#[test]
fn passes_the_configured_timeout_to_every_request() {
    let fixture = exporter_with(vec![Step::Status(200)], &[("LANGFUSE_TIMEOUT", "2.5")]);
    assert!(export(&fixture.exporter, vec![span("one", &[])]));
    assert_eq!(
        fixture.transport.seen()[0].timeout,
        Duration::from_millis(2500)
    );
}

#[test]
fn rejects_a_timeout_that_is_not_a_number() {
    let error = options_for(&[("LANGFUSE_TIMEOUT", "soon")]).unwrap_err();
    assert!(matches!(error, LangfuseExporterError::Settings(_)));
}

#[test]
fn the_secret_never_reaches_the_options_debug_output() {
    let options = options_for(&[]).unwrap();
    let debug = format!("{options:?}");
    assert!(!debug.contains("sk-lf-test"));
    assert!(debug.contains("pk-lf-test"));
}

#[rstest]
#[case::final_hop_accepts(204, true)]
#[case::final_hop_rejects(404, false)]
#[tokio::test]
async fn follows_a_308_with_the_same_body_and_headers(
    #[case] final_status: u16,
    #[case] delivered: bool,
) {
    let server = scripted(vec![
        reply(308, Some("/moved/traces")),
        reply(final_status, None),
    ])
    .await;
    let (exporter, _sleep) = http_exporter(&server.base);
    let result = exporter
        .export(vec![span("one", &[(INPUT, "hello")])])
        .await;
    assert_eq!(result.is_ok(), delivered);
    let seen = server.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].head.starts_with("POST /api/public/otel/v1/traces "));
    assert!(seen[1].head.starts_with("POST /moved/traces "));
    assert_eq!(seen[1].body, seen[0].body);
    assert_eq!(seen[1].sent_headers(), seen[0].sent_headers());
    assert_eq!(seen[1].header("x-langfuse-sdk-name"), Some("python"));
    assert_eq!(
        seen[1].header("authorization"),
        Some("Basic cGstbGYtdGVzdDpzay1sZi10ZXN0")
    );
}

fn http_exporter(base_url: &str) -> (LangfuseSpanExporter<HttpTransport>, Arc<RecordingSleep>) {
    let mut options = options_for(&[]).unwrap();
    options.base_url = base_url.to_owned();
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let transport = HttpTransport {
        client: ProxiedClient::outbound(&policy).unwrap(),
    };
    let sleep = Arc::new(RecordingSleep::default());
    let recording = Arc::clone(&sleep);
    let sleep_dependency: Arc<dyn Sleep> = recording;
    let exporter = LangfuseSpanExporter::new(
        &options,
        transport,
        Arc::new(OtlpSpanEncoder),
        sleep_dependency,
    )
    .unwrap();
    (exporter, sleep)
}

#[tokio::test]
async fn a_redirect_loop_is_one_failed_attempt_and_is_not_retried() {
    let server = scripted(vec![reply(308, Some("/loop")); 11]).await;
    let (exporter, sleep) = http_exporter(&server.base);
    assert!(exporter.export(vec![span("one", &[])]).await.is_err());
    assert_eq!(server.seen.lock().unwrap().len(), 11);
    assert!(sleep.seconds().is_empty());
}

#[tokio::test]
async fn a_connection_closed_without_a_reply_is_retried_with_the_backoff() {
    let server = scripted(Vec::new()).await;
    let (exporter, sleep) = http_exporter(&server.base);
    assert!(exporter.export(vec![span("one", &[])]).await.is_err());
    assert_eq!(sleep.seconds(), vec![1, 2, 4]);
}
