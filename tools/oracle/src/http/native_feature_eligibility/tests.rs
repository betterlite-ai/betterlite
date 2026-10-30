#![cfg(test)]

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use serde_json::Value;

use crate::case::{BodyKind, Case, Request, decode_http_case, validate};
use crate::cli::Options;
use crate::gateway::{self, GatewayInput};
use crate::http::{self, client};
use crate::process::Gateway;

fn case(extra: &str) -> String {
    let body = "name: scim\nconfig: |\n  general_settings: {}\n";
    format!(
        "{body}env: {{{extra}}}\nrequest: {{method: GET, path: /scim/v2/ServiceProviderConfig, headers: {{}}, body: null}}\nupstream: []\ncompare: {{headers: [], headers_present: [], upstream_headers: [], ignore: []}}\n"
    )
}

#[test]
fn absent_eligibility_is_stock_behavior() {
    let decoded = decode_http_case(case("").as_bytes()).expect("case decodes");
    assert!(!decoded.native_feature_eligibility);
}

#[test]
fn explicit_eligibility_decodes() {
    let decoded =
        decode_http_case(format!("native_feature_eligibility: true\n{}", case("")).as_bytes())
            .expect("case decodes");
    assert!(decoded.native_feature_eligibility);
}

#[rstest::rstest]
#[case("LITELLM_LICENSE")]
#[case("LITELLM_WORKER_STARTUP_HOOKS")]
fn controlled_environment_values_are_rejected(#[case] key: &str) {
    let result = decode_http_case(case(&format!("\n  {key}: value")).as_bytes());
    let _error = result.unwrap_err();
}

#[test]
fn controlled_config_license_is_rejected() {
    let source = case("").replace(
        "general_settings: {}",
        "general_settings: {litellm_license: value}",
    );
    let result = decode_http_case(source.as_bytes());
    let _error = result.unwrap_err();
}

#[test]
fn validation_accepts_unrelated_environment_and_config() {
    let source = case("\n  LITELLM_LOCAL_MODEL_COST_MAP: 'True'");
    let parsed: Case = serde_saphyr::from_str(&source).expect("case decodes");
    validate(Path::new("<test>"), &parsed).expect("unrelated settings are allowed");
    assert_eq!(
        parsed.env,
        BTreeMap::from([(
            String::from("LITELLM_LOCAL_MODEL_COST_MAP"),
            String::from("True")
        )])
    );
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn group_dir(suffix: &str) -> PathBuf {
    let directory = env::temp_dir().join(format!(
        "betterlite-oracle-native-feature-{}-{suffix}",
        process::id()
    ));
    fs::create_dir_all(&directory).expect("temporary group directory is created");
    directory
}

fn boot_scim_gateway(eligible: bool, suffix: &str) -> Gateway {
    let directory = group_dir(suffix);
    let config_path = directory.join("config.yaml");
    fs::write(&config_path, "general_settings: {}\n").expect("scim config is written");
    let mut case_env = BTreeMap::new();
    if eligible {
        let fixture = root().join("tools/oracle/fixtures/native-feature-oracle");
        case_env.insert(
            "LITELLM_WORKER_STARTUP_HOOKS".to_owned(),
            "startup:set_premium_user".to_owned(),
        );
        case_env.insert(
            "PYTHONPATH".to_owned(),
            fixture.to_str().expect("fixture path is UTF-8").to_owned(),
        );
    }
    let options = Options {
        self_check: true,
        paths: Vec::new(),
    };
    let input = GatewayInput {
        options: &options,
        group_dir: &directory,
        config_path: &config_path,
        case_env: &case_env,
    };
    gateway::start(&input, true).expect("the real litellm proxy starts")
}

fn scim_request(authorization: Option<&str>) -> Request {
    let mut headers = BTreeMap::new();
    if let Some(value) = authorization {
        headers.insert("Authorization".to_owned(), value.to_owned());
    }
    Request {
        method: "GET".to_owned(),
        path: "/scim/v2/ServiceProviderConfig".to_owned(),
        headers,
        body: Value::Null,
        body_kind: BodyKind::Json,
    }
}

#[test]
fn eligible_scim_service_provider_config_reaches_the_mit_handler() {
    let mut gateway = boot_scim_gateway(true, "eligible");
    let request_client = client().expect("an http client builds");
    let authenticated = http::send(
        &request_client,
        &gateway.base_url,
        &scim_request(Some("Bearer oracle-master")),
    )
    .expect("the eligible proxy answers");
    assert_eq!(authenticated.status, 200);
    assert_eq!(
        authenticated.body["meta"]["resourceType"],
        Value::from("ServiceProviderConfig")
    );
    let unauthenticated = http::send(&request_client, &gateway.base_url, &scim_request(None))
        .expect("the eligible proxy answers");
    // Same unstructured 500 as tools/oracle/cases/auth/missing-key.yaml; auth still ran.
    assert_eq!(unauthenticated.status, 500);
    gateway.stop().expect("the gateway stops");
}

#[test]
fn stock_scim_service_provider_config_keeps_the_pinned_rejection() {
    let mut gateway = boot_scim_gateway(false, "stock");
    let response = http::send(
        &client().expect("an http client builds"),
        &gateway.base_url,
        &scim_request(Some("Bearer oracle-master")),
    )
    .expect("the stock proxy answers");
    assert_eq!(response.status, 403);
    gateway.stop().expect("the gateway stops");
}

#[test]
fn concurrent_eligible_and_default_runs_do_not_leak_flags() {
    let mut eligible = boot_scim_gateway(true, "concurrent-eligible");
    let mut stock = boot_scim_gateway(false, "concurrent-stock");
    let request_client = client().expect("an http client builds");
    let eligible_response = http::send(
        &request_client,
        &eligible.base_url,
        &scim_request(Some("Bearer oracle-master")),
    )
    .expect("the eligible proxy answers");
    let stock_response = http::send(
        &request_client,
        &stock.base_url,
        &scim_request(Some("Bearer oracle-master")),
    )
    .expect("the stock proxy answers");
    assert_eq!(eligible_response.status, 200);
    assert_eq!(stock_response.status, 403);
    eligible.stop().expect("the gateway stops");
    stock.stop().expect("the gateway stops");
}

#[test]
fn dropping_the_owner_kills_the_actual_server_process() {
    let gateway = boot_scim_gateway(true, "drop");
    let pid = gateway.child.id();
    fs::metadata(format!("/proc/{pid}")).expect("the process is running");
    drop(gateway);
    fs::metadata(format!("/proc/{pid}")).expect_err("the process is gone");
}

#[rstest::rstest]
#[case::stock(false)]
#[case::eligible(true)]
fn enterprise_import_still_fails(#[case] eligible: bool) {
    let proprietary_module = format!("litellm{}", "_enterprise");
    let mut command = process::Command::new("uv");
    command
        .args(["run", "--locked", "--project"])
        .arg(root().join("tools/oracle/litellm"))
        .args(["python", "-c", &format!("import {proprietary_module}")]);
    if eligible {
        let fixture = root().join("tools/oracle/fixtures/native-feature-oracle");
        command
            .env("LITELLM_WORKER_STARTUP_HOOKS", "startup:set_premium_user")
            .env("PYTHONPATH", fixture);
    }
    let status = command.status().expect("uv runs");
    assert!(!status.success());
}
