#![cfg(test)]
use crate::OracleError;
use crate::case::Compare;
use crate::case::decode_case;
use crate::cli::Options;
use crate::compare::compare;
use crate::http::Response;
use crate::paths::unique_id;
use crate::runner::DatabaseEnvironment;
use crate::runner::run_group_with_url;
use betterlite_postgres_fixture::connection::RandomDatabaseName;
use betterlite_postgres_fixture::error::FixtureError;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn legacy_case_without_database() {
    let case = decode_case(Path::new("<bytes>"), b"name: legacy\nconfig: 'general_settings: {}'\nenv: {}\nrequest: {method: GET, path: /health/liveliness, headers: {}, body: null}\nupstream: []\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\n").unwrap();
    let options = Options {
        self_check: true,
        paths: Vec::new(),
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/oracle");
    let result =
        run_group_with_url(&options, &root, &[case], None, &mut RandomDatabaseName).unwrap();
    assert_eq!(result, [false]);
}

#[test]
fn public_route_without_key() {
    let case = decode_case(Path::new("<bytes>"), b"name: public\nconfig: 'general_settings: {}'\nenv: {}\nrequest: {method: GET, path: /health/liveliness, headers: {}, body: null}\nupstream: []\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\n")
    .unwrap();
    assert_eq!(case.request.path, "/health/liveliness");
    assert!(case.request.headers.is_empty());
    let options = Options {
        self_check: true,
        paths: Vec::new(),
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/oracle")
        .join(unique_id().unwrap());
    let result =
        run_group_with_url(&options, &root, &[case], None, &mut RandomDatabaseName).unwrap();
    assert_eq!(result, [false]);
    let captures = fs::read_dir(root.join("public"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(captures.len(), 1);
    let directory = captures.first().unwrap().path();
    let first: Response =
        serde_json::from_slice(&fs::read(directory.join("client-litellm.json")).unwrap()).unwrap();
    let second: Response =
        serde_json::from_slice(&fs::read(directory.join("client-betterlite.json")).unwrap())
            .unwrap();
    assert_eq!(first.status, 200);
    assert_eq!(second.status, 200);
}

#[test]
fn fixture_unknown_field() {
    let result = decode_case(
        Path::new("<bytes>"),
        include_bytes!("../../../fuzz/corpus/http_case/postgres-unknown-field.yaml"),
    );
    assert!(matches!(result, Err(OracleError::CaseParse { .. })));
}

#[test]
fn duplicate_identity() {
    let result = decode_case(
        Path::new("<bytes>"),
        include_bytes!("../../../fuzz/corpus/http_case/postgres-duplicate-identity.yaml"),
    );
    assert!(matches!(
        result,
        Err(OracleError::HttpCaseFixture {
            source: FixtureError::DuplicateIdentity { .. }
        })
    ));
}

#[test]
fn missing_principal() {
    let result = decode_case(
        Path::new("<bytes>"),
        include_bytes!("../../../fuzz/corpus/http_case/postgres-missing-principal.yaml"),
    );
    assert!(matches!(
        result,
        Err(OracleError::HttpCaseFixture {
            source: FixtureError::MissingReference { .. }
        })
    ));
}

#[test]
fn malformed_yaml() {
    let result = decode_case(
        Path::new("<bytes>"),
        include_bytes!("../../../fuzz/corpus/http_case/postgres-malformed.yaml"),
    );
    assert!(matches!(result, Err(OracleError::CaseParse { .. })));
}

#[test]
fn truncated_rows() {
    let result = decode_case(
        Path::new("<bytes>"),
        include_bytes!("../../../fuzz/corpus/http_case/postgres-truncated-rows.yaml"),
    );
    assert!(matches!(result, Err(OracleError::CaseParse { .. })));
}

#[test]
fn empty_configured_url_is_kept_for_the_fixture() {
    let environment = <DatabaseEnvironment as clap::Parser>::try_parse_from([
        "oracle",
        "--postgres-test-url",
        "",
    ])
    .unwrap();
    let url = environment.postgres_test_url.unwrap();
    assert_eq!(url.expose_secret(), "");
}

#[test]
fn replays_the_http_case_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/http_case");
    // a seed folder under http_case belongs to the fixture extension that replays it
    let mut paths = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .try_for_each(|path| {
            let bytes = fs::read(path.as_path()).unwrap();
            let result = decode_case(Path::new("<bytes>"), &bytes);
            let name = path.file_name().and_then(|value| value.to_str()).unwrap();
            match name {
                "postgres-valid.yaml" => assert!(result.is_ok()),
                "postgres-unknown-field.yaml"
                | "postgres-malformed.yaml"
                | "postgres-truncated-rows.yaml" => {
                    assert!(matches!(result, Err(OracleError::CaseParse { .. })));
                }
                "postgres-duplicate-identity.yaml" => assert!(matches!(
                    result,
                    Err(OracleError::HttpCaseFixture {
                        source: FixtureError::DuplicateIdentity { .. }
                    })
                )),
                "postgres-missing-principal.yaml" => assert!(matches!(
                    result,
                    Err(OracleError::HttpCaseFixture {
                        source: FixtureError::MissingReference { .. }
                    })
                )),
                _ => assert!(result.is_err(), "{} should be rejected", path.display()),
            }
            Ok::<(), ()>(())
        })
        .unwrap();
}

#[test]
fn detects_status_mismatch() {
    let left = response(200, body("{\"ok\":true}"));
    let right = response(500, body("{\"ok\":true}"));
    assert_eq!(
        compare(&left, &right, &[], &[], &Compare::default())
            .unwrap_err()
            .pointer,
        "/status"
    );
}

#[test]
fn detects_error_param_mismatch() {
    let left = response(400, body("{\"error\":{\"param\":\"model\"}}"));
    let right = response(400, body("{\"error\":{\"param\":\"messages\"}}"));
    assert_eq!(
        compare(&left, &right, &[], &[], &Compare::default())
            .unwrap_err()
            .pointer,
        "/error/param"
    );
}

#[test]
fn detects_role_mismatch() {
    let left = response(200, body("{\"principal\":{\"role\":\"proxy_admin\"}}"));
    let right = response(200, body("{\"principal\":{\"role\":\"internal_user\"}}"));
    assert_eq!(
        compare(&left, &right, &[], &[], &Compare::default())
            .unwrap_err()
            .pointer,
        "/principal/role"
    );
}

fn response(status: u16, body: serde_json::Value) -> Response {
    Response {
        status,
        headers: BTreeMap::new(),
        body,
    }
}

fn body(source: &str) -> serde_json::Value {
    serde_json::from_str(source).unwrap()
}

// LiteLLM reads a key through the project view, so these cases take the fixture's verified chain, not the 2025-03 baseline a plain seed applies
#[test]
fn independent_initial_rows() {
    let case = decode_case(
        Path::new("<bytes>"),
        br#"name: independent-initial-rows
config: 'general_settings: {master_key: oracle-master}'
env: {}
request: {method: POST, path: /user/new, headers: {Authorization: 'Bearer sk-fixture-admin'}, body: {user_id: fixture-seeded, user_role: internal_user, auto_create_key: false}}
upstream: []
database: {seed: {dashboard: {logs: []}, users: [{user_id: fixture-admin}, {user_id: fixture-seeded}], keys: [{token: sk-fixture-admin, user_id: fixture-admin, role: proxy_admin}]}}
compare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}
"#,
    )
    .unwrap();
    let options = Options {
        self_check: true,
        paths: Vec::new(),
    };
    let root = unique_root("independent-initial-rows");
    let url = postgres_url();
    let results = run_group_with_url(
        &options,
        &root,
        &[case],
        Some(&url),
        &mut RandomDatabaseName,
    )
    .unwrap();
    assert_eq!(results, [false]);
    let (first, second) = capture_pair(&root, "independent-initial-rows");
    assert!(!(200..300).contains(&first.status));
    assert_eq!(second.status, first.status);
    assert_eq!(second.body, first.body);
}

#[test]
fn first_gateway_write_not_shared() {
    let case = decode_case(
        Path::new("<bytes>"),
        br#"name: first-gateway-write-not-shared
config: 'general_settings: {master_key: oracle-master}'
env: {}
request: {method: POST, path: /user/new, headers: {Authorization: 'Bearer sk-fixture-admin'}, body: {user_id: fixture-created, user_role: internal_user, auto_create_key: false}}
upstream: []
database: {seed: {dashboard: {logs: []}, users: [{user_id: fixture-admin}], keys: [{token: sk-fixture-admin, user_id: fixture-admin, role: proxy_admin}]}}
compare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}
"#,
    )
    .unwrap();
    let options = Options {
        self_check: true,
        paths: Vec::new(),
    };
    let root = unique_root("first-gateway-write-not-shared");
    let url = postgres_url();
    let results = run_group_with_url(
        &options,
        &root,
        &[case],
        Some(&url),
        &mut RandomDatabaseName,
    )
    .unwrap();
    assert_eq!(results, [false]);
    let (first, second) = capture_pair(&root, "first-gateway-write-not-shared");
    assert_eq!(first.status, 200);
    assert_eq!(second.status, 200);
}

const HASHED_ADMIN_CASE: &str = r#"name: hashed-admin-creates-user
config: 'general_settings: {master_key: oracle-master}'
env: {}
request: {method: POST, path: /user/new, headers: {Authorization: 'Bearer sk-fixture-admin'}, body: {user_id: fixture-created, user_role: internal_user, auto_create_key: false}}
upstream: []
database: {seed: {dashboard: {logs: []}, users: [{user_id: fixture-admin}, {user_id: fixture-internal}], keys: [{token: sk-fixture-admin, user_id: fixture-admin, role: proxy_admin}, {token: sk-fixture-internal, user_id: fixture-internal, role: internal_user}]}}
compare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}
"#;

const HASHED_INTERNAL_CASE: &str = r#"name: hashed-internal-key-denied
config: 'general_settings: {master_key: oracle-master}'
env: {}
request: {method: POST, path: /user/new, headers: {Authorization: 'Bearer sk-fixture-internal'}, body: {user_id: fixture-denied, user_role: internal_user, auto_create_key: false}}
upstream: []
database: {seed: {dashboard: {logs: []}, users: [{user_id: fixture-admin}, {user_id: fixture-internal}], keys: [{token: sk-fixture-admin, user_id: fixture-admin, role: proxy_admin}, {token: sk-fixture-internal, user_id: fixture-internal, role: internal_user}]}}
compare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}
"#;

#[test]
fn hashed_fixture_keys_and_roles() {
    let admin = decode_case(Path::new("<bytes>"), HASHED_ADMIN_CASE.as_bytes()).unwrap();
    let internal = decode_case(Path::new("<bytes>"), HASHED_INTERNAL_CASE.as_bytes()).unwrap();
    let options = Options {
        self_check: true,
        paths: Vec::new(),
    };
    let root = unique_root("hashed-fixture-keys-and-roles");
    let url = postgres_url();
    let results = run_group_with_url(
        &options,
        &root,
        &[admin, internal],
        Some(&url),
        &mut RandomDatabaseName,
    )
    .unwrap();
    assert_eq!(results, [false, false]);
    let (admin_first, admin_second) = capture_pair(&root, "hashed-admin-creates-user");
    assert_eq!(admin_first.status, 200);
    assert_eq!(admin_second.status, 200);
    let (internal_first, internal_second) = capture_pair(&root, "hashed-internal-key-denied");
    assert!(!(200..300).contains(&internal_first.status));
    assert_eq!(internal_second.status, internal_first.status);
    assert_eq!(internal_second.body, internal_first.body);
}

#[test]
fn disabled_schema_update_still_checks_current_schema() {
    let case = decode_case(
        Path::new("<bytes>"),
        br#"name: disabled-schema-update
config: 'general_settings: {master_key: oracle-master}'
env: {}
request: {method: GET, path: /health/liveliness, headers: {}, body: null}
upstream: []
database: {seed: {}}
compare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}
"#,
    )
    .unwrap();
    let options = Options {
        self_check: true,
        paths: Vec::new(),
    };
    let root = unique_root("disabled-schema-update");
    let url = postgres_url();
    let results = run_group_with_url(
        &options,
        &root,
        &[case],
        Some(&url),
        &mut RandomDatabaseName,
    )
    .unwrap();
    assert_eq!(results, [false]);
    let (first, second) = capture_pair(&root, "disabled-schema-update");
    assert_eq!(first.status, 200);
    assert_eq!(second.status, 200);
    let log = group_log(&root);
    let failed_diff = log.contains("Failed to generate migration diff");
    let changed_schema = log.contains("Changes to DB Schema detected");
    // the pinned CLI prints "Failed to generate migration diff", because tools/oracle/litellm has no ./schema.prisma
    assert!(
        failed_diff && !changed_schema,
        "Failed to generate migration diff={failed_diff}, Changes to DB Schema detected={changed_schema}"
    );
}

fn postgres_url() -> SecretString {
    let environment =
        <DatabaseEnvironment as clap::Parser>::try_parse_from([env!("CARGO_PKG_NAME")])
            .expect("database environment parses");
    environment
        .postgres_test_url
        .expect("POSTGRES_TEST_URL is set for this test")
}

fn unique_root(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/oracle")
        .join(name)
        .join(unique_id().unwrap())
}

fn capture_pair(root: &Path, case_name: &str) -> (Response, Response) {
    let captures = fs::read_dir(root.join(case_name))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(captures.len(), 1);
    let directory = captures.first().unwrap().path();
    let first: Response =
        serde_json::from_slice(&fs::read(directory.join("client-litellm.json")).unwrap()).unwrap();
    let second: Response =
        serde_json::from_slice(&fs::read(directory.join("client-betterlite.json")).unwrap())
            .unwrap();
    (first, second)
}

fn group_log(root: &Path) -> String {
    let group = fs::read_dir(root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("group-"))
        })
        .expect("the run writes one group folder");
    fs::read_to_string(group.join("litellm.log")).unwrap()
}
