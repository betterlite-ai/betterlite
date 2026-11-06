use std::io::Read as _;

use betterlite_logging::focus_records::destinations::FocusDestination as _;
use betterlite_logging::mavvrik_delivery::error::MavvrikDeliveryError;
use betterlite_logging::mavvrik_delivery::transport::Method;
use flate2::read::GzDecoder;
use rstest::rstest;

use super::support::{
    AGENT_URL, SESSION_URI, SIGNED_URL, Step, block_on, destination, noise, registered, respond,
    respond_with_location, signed_url_found, window,
};

const CHUNK: usize = 8 * 1024 * 1024;

fn session_started() -> Vec<Step> {
    vec![
        registered(),
        signed_url_found(),
        respond_with_location(200, SESSION_URI),
    ]
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    GzDecoder::new(bytes).read_to_end(&mut decoded).unwrap();
    decoded
}

#[test]
fn csv_content_is_gzipped_into_one_chunk_with_a_closed_content_range() {
    let content = b"a,b\n1,2\n".repeat(100);
    let mut steps = session_started();
    steps.extend([respond(200, ""), respond(200, "")]);
    let (destination, transport) = destination(steps);
    block_on(destination.deliver(&content, &window(), "export.csv")).unwrap();

    let calls = transport.drain();
    let order: Vec<_> = calls
        .iter()
        .map(|call| (call.method, call.url.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            (Method::Post, AGENT_URL),
            (
                Method::Get,
                concat!(
                    "https://api.mavvrik.dev/tenant/metrics/agent/ai/conn-1/upload-url",
                    "?name=2024-03-05&type=metrics&datetime=2024-03-05"
                )
            ),
            (Method::Post, SIGNED_URL),
            (Method::Put, SESSION_URI),
            (Method::Patch, AGENT_URL),
        ]
    );
    let put = &calls[3];
    assert_eq!(gunzip(&put.body), content);
    let range = format!("bytes 0-{}/{}", put.body.len() - 1, put.body.len());
    assert!(put.headers.contains(&("Content-Range", range)));
    assert!(
        put.headers
            .contains(&("Content-Type", "application/gzip".to_owned()))
    );
    assert_eq!(put.api_key, None);
    assert_eq!(put.timeout_secs, 120);
}

#[test]
fn the_session_is_started_with_the_gzip_metadata_and_the_resumable_header() {
    let mut steps = session_started();
    steps.extend([respond(200, ""), respond(200, "")]);
    let (destination, transport) = destination(steps);
    block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap();
    let init = &transport.drain()[2];
    assert_eq!(
        init.body,
        br#"{"contentEncoding":"gzip","contentDisposition":"attachment"}"#
    );
    assert!(
        init.headers
            .contains(&("x-goog-resumable", "start".to_owned()))
    );
    assert!(
        init.headers
            .contains(&("Content-Type", "application/gzip".to_owned()))
    );
    assert_eq!(init.api_key, None);
    assert_eq!(init.timeout_secs, 30);
}

#[test]
fn a_large_export_goes_in_8_mib_chunks_with_open_ranges_until_the_last() {
    let content = noise(CHUNK + 1024 * 1024);
    let mut steps = session_started();
    steps.extend([respond(308, ""), respond(200, ""), respond(200, "")]);
    let (destination, transport) = destination(steps);
    block_on(destination.deliver(&content, &window(), "export.csv")).unwrap();

    let calls = transport.drain();
    assert_eq!(
        calls.iter().map(|call| call.method).collect::<Vec<_>>(),
        [
            Method::Post,
            Method::Get,
            Method::Post,
            Method::Put,
            Method::Put,
            Method::Patch
        ]
    );
    let (first, second) = (&calls[3], &calls[4]);
    assert_eq!(first.body.len(), CHUNK);
    assert!(
        first
            .headers
            .contains(&("Content-Range", format!("bytes 0-{}/*", CHUNK - 1)))
    );
    let total = CHUNK + second.body.len();
    let last_range = format!("bytes {CHUNK}-{}/{total}", total - 1);
    assert!(second.headers.contains(&("Content-Range", last_range)));
    assert!(second.body.len() < CHUNK);
    assert_eq!(
        [first.body.clone(), second.body.clone()].concat().len(),
        total
    );
    assert_eq!(
        gunzip(&[first.body.clone(), second.body.clone()].concat()),
        content
    );
    assert_eq!((first.timeout_secs, second.timeout_secs), (120, 120));
}

#[test]
fn the_final_chunk_may_answer_201() {
    let mut steps = session_started();
    steps.extend([respond(201, ""), respond(200, "")]);
    let (destination, transport) = destination(steps);
    block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap();
    assert_eq!(transport.methods().last(), Some(&Method::Patch));
}

#[test]
fn the_marker_is_advanced_only_after_the_final_chunk_succeeds() {
    let mut steps = session_started();
    steps.extend([respond(500, "storage down"), respond(204, "")]);
    let (destination, transport) = destination(steps);
    block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap_err();
    assert!(!transport.methods().contains(&Method::Patch));
}

#[test]
fn an_intermediate_chunk_answered_200_is_an_error() {
    let mut steps = session_started();
    steps.extend([respond(200, "early"), respond(204, "")]);
    let (destination, _transport) = destination(steps);
    let error =
        block_on(destination.deliver(&noise(CHUNK + 4 * 1024 * 1024), &window(), "export.csv"))
            .unwrap_err();
    assert!(
        matches!(
            &error,
            MavvrikDeliveryError::ChunkFailed {
                offset: 0,
                expected: "{308}",
                status: 200,
                ..
            }
        ),
        "{error:?}"
    );
}

#[rstest]
#[case::continue_status(308)]
#[case::no_content(204)]
fn a_final_chunk_outside_200_and_201_is_an_error(#[case] status: u16) {
    let mut steps = session_started();
    steps.extend([respond(status, "unexpected"), respond(204, "")]);
    let (destination, _transport) = destination(steps);
    let error = block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap_err();
    assert!(
        matches!(&error, MavvrikDeliveryError::ChunkFailed { offset: 0, expected: "{200, 201}", status: got, .. } if *got == status),
        "{error:?}"
    );
}

#[test]
fn a_failed_chunk_reports_the_first_400_bytes_of_the_body() {
    let mut steps = session_started();
    steps.extend([respond(503, &"c".repeat(600)), respond(204, "")]);
    let (destination, _transport) = destination(steps);
    let error = block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap_err();
    assert!(
        matches!(&error, MavvrikDeliveryError::ChunkFailed { status: 503, body, .. } if *body == "c".repeat(400)),
        "{error:?}"
    );
}

#[rstest]
#[case::client_error(403)]
#[case::server_error(500)]
#[case::no_content(204)]
fn a_session_init_outside_200_and_201_fails_with_the_first_400_bytes(#[case] status: u16) {
    let (destination, transport) = destination(vec![
        registered(),
        signed_url_found(),
        respond(status, &"d".repeat(600)),
    ]);
    let error = block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap_err();
    assert!(
        matches!(&error, MavvrikDeliveryError::SessionInitFailed { status: got, body } if *got == status && *body == "d".repeat(400)),
        "{error:?}"
    );
    assert_eq!(
        transport.methods(),
        [Method::Post, Method::Get, Method::Post]
    );
}

#[test]
fn a_session_init_answered_201_is_accepted() {
    let (destination, transport) = destination(vec![
        registered(),
        signed_url_found(),
        respond_with_location(201, SESSION_URI),
        respond(200, ""),
        respond(200, ""),
    ]);
    block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap();
    assert_eq!(transport.methods().len(), 5);
}

#[rstest]
#[case::absent(respond(200, ""))]
#[case::empty(respond_with_location(200, ""))]
fn a_session_init_without_a_location_is_an_error(#[case] init: Step) {
    let (destination, _transport) = destination(vec![registered(), signed_url_found(), init]);
    let error = block_on(destination.deliver(b"data", &window(), "export.csv")).unwrap_err();
    assert!(
        matches!(error, MavvrikDeliveryError::SessionLocationMissing),
        "{error:?}"
    );
}
