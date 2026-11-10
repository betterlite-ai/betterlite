use std::future::Future;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
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
use crate::langfuse_exporter::endpoint::endpoint;
use crate::langfuse_exporter::error::LangfuseExporterError;
use crate::langfuse_exporter::exporter::LangfuseSpanExporter;

use super::support::{
    CLOUD_ENDPOINT, DecodedSpan, ScriptedTransport, Step, UNENCODABLE_KEY, attribute, captured,
    exporter_with, header, options_for, rejecting_exporter_with, span, spans_in, spans_of,
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
#[case::cloud_default(None, None, CLOUD_ENDPOINT)]
#[case::empty_base(
    None,
    Some("/custom/traces"),
    "https://cloud.langfuse.com/custom/traces"
)]
#[case::trailing_base_and_leading_path(
    Some("https://langfuse.example/"),
    Some("/custom/traces"),
    "https://langfuse.example/custom/traces"
)]
#[case::neither_side_slashed(
    Some("https://langfuse.example"),
    Some("custom/traces"),
    "https://langfuse.example/custom/traces"
)]
#[case::many_slashes(
    Some("https://langfuse.example///"),
    Some("///custom/traces"),
    "https://langfuse.example/custom/traces"
)]
#[case::empty_path_uses_the_default(
    Some("https://langfuse.example"),
    Some(""),
    "https://langfuse.example/api/public/otel/v1/traces"
)]
fn joins_the_base_url_and_the_export_path(
    #[case] base_url: Option<&str>,
    #[case] export_path: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(endpoint(base_url, export_path), expected);
}

#[test]
fn requests_the_export_path_from_the_environment() {
    let fixture = exporter_with(
        vec![Step::Status(200)],
        &[("LANGFUSE_OTEL_TRACES_EXPORT_PATH", "/custom/otel/traces")],
    );
    assert!(export(&fixture.exporter, vec![span("one", &[])]));
    let seen = fixture.transport.seen();
    assert_eq!(seen[0].url, "https://cloud.langfuse.com/custom/otel/traces");
}

#[test]
fn sends_the_basic_authorization_and_every_sdk_header() {
    let fixture = exporter_with(vec![Step::Status(200)], &[]);
    assert!(export(&fixture.exporter, vec![span("one", &[])]));
    let pairs = &fixture.transport.seen()[0].headers;
    let expected = format!("Basic {}", STANDARD.encode("pk-lf-test:sk-lf-test"));
    assert_eq!(pairs.len(), 6);
    assert_eq!(header(pairs, "authorization"), Some(expected));
    assert_eq!(
        header(pairs, "content-type").as_deref(),
        Some("application/x-protobuf")
    );
    assert_eq!(
        header(pairs, "x-langfuse-sdk-name").as_deref(),
        Some("python")
    );
    assert_eq!(
        header(pairs, "x-langfuse-sdk-version").as_deref(),
        Some("4.15.2")
    );
    assert_eq!(
        header(pairs, "x-langfuse-public-key").as_deref(),
        Some("pk-lf-test")
    );
    assert_eq!(
        header(pairs, "x-langfuse-ingestion-version").as_deref(),
        Some("4")
    );
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
#[case::redirect(302)]
#[case::no_content(204)]
fn accepts_any_status_httpx_would_not_raise_for(#[case] status: u16) {
    let fixture = exporter_with(vec![Step::Status(status)], &[]);
    assert!(export(&fixture.exporter, vec![span("one", &[])]));
    assert_eq!(fixture.transport.seen().len(), 1);
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
fn drops_a_span_with_no_langfuse_field_to_truncate() {
    let fixture = exporter_with(vec![Step::Status(413)], &[]);
    let batch = vec![span("one", &[("other.field", "value")])];
    assert!(!export(&fixture.exporter, batch));
    assert_eq!(fixture.transport.seen().len(), 1);
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
fn an_empty_batch_is_sent_once_as_litellm_sends_it() {
    let fixture = exporter_with(vec![Step::Status(200)], &[]);
    assert!(export(&fixture.exporter, Vec::new()));
    let seen = fixture.transport.seen();
    assert_eq!(seen.len(), 1);
    assert!(spans_in(&seen[0].body).is_empty());
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
    resource
        .attributes
        .iter()
        .find(|attribute| attribute.key == name)
        .map(|attribute| {
            attribute
                .value
                .as_ref()
                .and_then(|value| value.value.as_ref())
                .map_or_else(String::new, |value| match value {
                    AnyValueKind::StringValue(text) => text.clone(),
                    AnyValueKind::BoolValue(_)
                    | AnyValueKind::IntValue(_)
                    | AnyValueKind::DoubleValue(_)
                    | AnyValueKind::ArrayValue(_)
                    | AnyValueKind::KvlistValue(_)
                    | AnyValueKind::BytesValue(_)
                    | AnyValueKind::StringValueStrindex(_) => String::new(),
                })
        })
}

#[test]
fn the_discarding_exporter_accepts_spans_without_a_transport() {
    let exporter = DiscardingSpanExporter;
    run(exporter.export(vec![span("one", &[])])).unwrap();
    exporter.force_flush().unwrap();
    exporter.shutdown().unwrap();
}

#[test]
fn a_failed_batch_reports_an_export_error() {
    let fixture = exporter_with(vec![Step::Status(400)], &[]);
    assert!(run(fixture.exporter.export(vec![span("one", &[])])).is_err());
    fixture.exporter.force_flush().unwrap();
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
    let error = options_for(&[("LANGFUSE_TIMEOUT", "soon")], None).unwrap_err();
    assert!(matches!(error, LangfuseExporterError::Settings(_)));
}

#[test]
fn rejects_a_timeout_below_zero() {
    let error = options_for(&[("LANGFUSE_TIMEOUT", "-1")], None).unwrap_err();
    assert!(matches!(
        error,
        LangfuseExporterError::UnusableTimeout { .. }
    ));
}

#[test]
fn the_secret_never_reaches_the_options_debug_output() {
    let options = options_for(&[], None).unwrap();
    let debug = format!("{options:?}");
    assert!(!debug.contains("sk-lf-test"));
    assert!(debug.contains("pk-lf-test"));
}
