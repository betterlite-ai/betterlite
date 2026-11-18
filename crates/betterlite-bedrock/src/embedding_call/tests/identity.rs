use super::fixture::{Probe, Reply, Run, Source, run, run_with_source};
use super::support::titan_body;

const ONE_ACCESS: &str = "AKIAIDENTITYONE00000";
const TWO_ACCESS: &str = "AKIAIDENTITYTWO00000";

pub(super) fn embedding_call_credential_identity_isolation() {
    distinct_sources_do_not_share_credentials();
    each_call_resolves_its_own_region();
}

fn distinct_sources_do_not_share_credentials() {
    let probe = probe();
    let first_source = Source::new(ONE_ACCESS, "first-secret", Some("first-token"));
    let second_source = Source::new(TWO_ACCESS, "second-secret", Some("second-token"));
    let first = run_with_source(&probe, &first_source);
    let second = run_with_source(&probe, &second_source);
    assert_eq!(first.headers()["X-Amz-Security-Token"], "first-token");
    assert_eq!(second.headers()["X-Amz-Security-Token"], "second-token");
    let first_auth = &first.headers()["Authorization"];
    let second_auth = &second.headers()["Authorization"];
    assert!(first_auth.starts_with(&format!(
        "AWS4-HMAC-SHA256 Credential={ONE_ACCESS}/20231114/us-east-1/bedrock/aws4_request"
    )));
    assert!(second_auth.starts_with(&format!(
        "AWS4-HMAC-SHA256 Credential={TWO_ACCESS}/20231114/us-east-1/bedrock/aws4_request"
    )));
    assert_ne!(first_auth, second_auth);
    assert_eq!(
        first.headers()["X-Amz-Date"],
        second.headers()["X-Amz-Date"]
    );
    let repeated = run_with_source(&probe, &first_source);
    assert_eq!(repeated.headers()["Authorization"], first_auth);
    assert_eq!(first.regions, ["us-east-1"]);
    assert_eq!(second.regions, ["us-east-1"]);
}

fn each_call_resolves_its_own_region() {
    let east = regional("us-east-1");
    let west = regional("eu-west-1");
    assert_eq!(east.regions, ["us-east-1"]);
    assert_eq!(west.regions, ["eu-west-1"]);
    assert!(east.posted().url.contains("bedrock-runtime.us-east-1."));
    assert!(west.posted().url.contains("bedrock-runtime.eu-west-1."));
    assert_ne!(
        east.headers()["Authorization"],
        west.headers()["Authorization"]
    );
}

fn probe() -> Probe {
    Probe {
        replies: vec![Reply::ok(&titan_body("[1.0]", 1))],
        ..Probe::default()
    }
}

fn regional(region: &'static str) -> Run {
    let probe = Probe {
        optional_params: vec![
            ("aws_access_key_id", "\"AKIAIOSFODNN7EXAMPLE\""),
            (
                "aws_secret_access_key",
                "\"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\"",
            ),
        ],
        environment: vec![("AWS_REGION", region)],
        replies: vec![Reply::ok(&titan_body("[1.0]", 1))],
        ..Probe::default()
    };
    run(&probe)
}
