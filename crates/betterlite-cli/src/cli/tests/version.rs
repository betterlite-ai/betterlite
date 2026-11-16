use std::thread;

use crate::cli::tests::support::{Answer, FakeEnvironment, Harness, serve, unused_base_url};

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

#[test]
fn version_prints_cli_url_and_server_versions() {
    let answer = Answer {
        status: 200,
        reason: "OK",
        body: "{\"litellm_version\":\"9.9.9\"}",
    };
    thread::scope(|scope| {
        let served = serve(scope, answer);
        let mut harness = Harness::new(FakeEnvironment::default());
        let code = harness.run(&["--base-url", &served.base_url, "version"]);
        assert_eq!(code, 0);
        let stdout = harness.stdout();
        assert!(stdout.contains(&format!("betterlite Proxy CLI Version: {CLI_VERSION}")));
        assert!(stdout.contains(&format!("betterlite Proxy Server URL: {}", served.base_url)));
        assert!(stdout.contains("betterlite Proxy Server Version: 9.9.9"));
        assert!(harness.stderr().is_empty());
    });
}

#[test]
fn version_flag_prints_the_cli_version_and_exits_zero() {
    let base_url = unused_base_url();
    let mut harness = Harness::new(FakeEnvironment::default());
    let code = harness.run(&["--version", "--base-url", &base_url]);
    assert_eq!(code, 0);
    let stdout = harness.stdout();
    assert!(stdout.starts_with(&format!("betterlite Proxy CLI Version: {CLI_VERSION}\n")));
    assert!(stdout.contains(&format!("betterlite Proxy Server URL: {base_url}")));
}

#[test]
fn short_version_flag_matches_the_long_one() {
    let base_url = unused_base_url();
    let mut harness = Harness::new(FakeEnvironment::default());
    let code = harness.run(&["-v", "--base-url", &base_url]);
    assert_eq!(code, 0);
    assert!(
        harness
            .stdout()
            .starts_with(&format!("betterlite Proxy CLI Version: {CLI_VERSION}\n"))
    );
}

#[test]
fn unreachable_server_version_is_reported_without_failing() {
    let base_url = unused_base_url();
    let mut harness = Harness::new(FakeEnvironment::default());
    let code = harness.run(&["--base-url", &base_url, "version"]);
    assert_eq!(code, 0);
    assert!(
        harness
            .stdout()
            .contains("Could not retrieve server version: ")
    );
}

#[test]
fn non_success_readiness_status_is_reported_without_failing() {
    let answer = Answer {
        status: 500,
        reason: "Internal Server Error",
        body: "{}",
    };
    thread::scope(|scope| {
        let served = serve(scope, answer);
        let mut harness = Harness::new(FakeEnvironment::default());
        let code = harness.run(&["--base-url", &served.base_url, "version"]);
        assert_eq!(code, 0);
        assert!(harness.stdout().contains("the proxy answered HTTP 500"));
    });
}

#[test]
fn missing_server_version_prints_unavailable() {
    let answer = Answer {
        status: 200,
        reason: "OK",
        body: "{}",
    };
    thread::scope(|scope| {
        let served = serve(scope, answer);
        let mut harness = Harness::new(FakeEnvironment::default());
        let code = harness.run(&["--base-url", &served.base_url, "version"]);
        assert_eq!(code, 0);
        assert!(
            harness
                .stdout()
                .contains("betterlite Proxy Server Version: (unavailable)")
        );
    });
}
