use super::super::ContainerRef;
use super::support::{block_on, cached_handle, environment, run_request, sandbox};

const OUTPUT: &str = "data: {\"type\":\"stdout\",\"text\":\"ok\\n\"}\n";

#[test]
fn handle_debug_redacts_the_endpoint_headers() {
    let handle = cached_handle("http://stored.example", "stored-key", "http://execd.local");
    let printed = format!("{handle:?}");

    assert!(printed.contains("[REDACTED]"));
    assert!(!printed.contains("execd-token"));
    assert!(!printed.contains("stored-key"));
}

#[test]
fn the_execution_request_hides_the_endpoint_header_value() {
    let sandbox = sandbox(&[(200, OUTPUT)], environment(&[]));
    let handle = cached_handle("http://stored.example", "stored-key", "http://execd.local");
    block_on(sandbox.arun_code(run_request(ContainerRef::Cached(&handle), "1"), None)).unwrap();

    let printed = format!("{:?}", sandbox.http.request(0).headers);
    assert!(printed.contains("Sensitive"));
    assert!(!printed.contains("execd-token"));
}
