#![cfg(test)]
use crate::OracleError;
use crate::case::Compare;
use crate::case::HttpCase;
use crate::case::decode_case;
use crate::cli::Options;
use crate::compare::compare;
use crate::http::Response;
use crate::paths::unique_id;
use crate::runner::DatabaseEnvironment;
use crate::runner::run_group_with_url;
use async_trait::async_trait;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Request, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_core::api::genai_chat_output::GenAiOutputError;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_openai::chat::{ChatMessage, MessageContent, Role as ChatRole};
use betterlite_oracle_runner::case::UpstreamAnswer;
use betterlite_postgres_fixture::connection::RandomDatabaseName;
use betterlite_postgres_fixture::error::FixtureError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_server::api::genai_routes::ports::{
    GenaiRoutesState, GenerateContentCall, GenerateContentDispatch, GenerationAnswer,
    GenerationBody, TokenCountOriginal, TokenCountRequest, TokenCountResponse, TokenCounter,
    TranslateContents,
};
use betterlite_server::api::genai_routes::routes;
use betterlite_server::auth::auth_identity::AuthContext;
use futures_util::stream::{self, StreamExt as _};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use tower::ServiceExt as _;

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

#[tokio::test]
async fn replays_the_http_case_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/http_case");
    // a seed folder under http_case belongs to the fixture extension that replays it
    let mut paths = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    paths.sort();
    stream::iter(paths)
        .for_each(|path| async move {
            let bytes = fs::read(path.as_path()).unwrap();
            let result = decode_case(Path::new("<bytes>"), &bytes);
            let name = path.file_name().and_then(|value| value.to_str()).unwrap();
            replay_seed(name, result).await;
        })
        .await;
}

// the seed's file name names the fixture extension that replays it, and an unknown name must not decode
async fn replay_seed(name: &str, result: Result<HttpCase, OracleError>) {
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
        "genai-generate-content.yaml"
        | "genai-generate-content-body-model.yaml"
        | "genai-generate-content-provider-error.yaml"
        | "genai-stream-generate-content.yaml"
        | "genai-count-tokens.yaml"
        | "genai-count-tokens-unauthenticated.yaml" => {
            replay_genai_seed(result, Counted::Original).await;
        }
        "genai-count-tokens-total.yaml" => replay_genai_seed(result, Counted::Total(41)).await,
        "genai-count-tokens-no-reply.yaml" => replay_genai_seed(result, Counted::None).await,
        _ => assert!(result.is_err(), "{name} should be rejected"),
    }
}

// which answer the internal counter gives on a count seed: the provider's own reply, a bare total, or nothing at all
#[derive(Clone, Copy)]
enum Counted {
    Original,
    Total(i64),
    None,
}

async fn replay_genai_seed(result: Result<HttpCase, OracleError>, counted: Counted) {
    let case = result.expect("a genai corpus seed decodes");
    replay_genai_case(&case, counted).await;
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
        b"name: independent-initial-rows
config: 'general_settings: {master_key: oracle-master}'
env: {}
request: {method: POST, path: /user/new, headers: {Authorization: 'Bearer sk-fixture-admin'}, body: {user_id: fixture-seeded, user_role: internal_user, auto_create_key: false}}
upstream: []
database: {seed: {dashboard: {logs: []}, users: [{user_id: fixture-admin}, {user_id: fixture-seeded}], keys: [{token: sk-fixture-admin, user_id: fixture-admin, role: proxy_admin}]}}
compare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}
",
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
        b"name: first-gateway-write-not-shared
config: 'general_settings: {master_key: oracle-master}'
env: {}
request: {method: POST, path: /user/new, headers: {Authorization: 'Bearer sk-fixture-admin'}, body: {user_id: fixture-created, user_role: internal_user, auto_create_key: false}}
upstream: []
database: {seed: {dashboard: {logs: []}, users: [{user_id: fixture-admin}], keys: [{token: sk-fixture-admin, user_id: fixture-admin, role: proxy_admin}]}}
compare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}
",
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

const HASHED_ADMIN_CASE: &str = "name: hashed-admin-creates-user
config: 'general_settings: {master_key: oracle-master}'
env: {}
request: {method: POST, path: /user/new, headers: {Authorization: 'Bearer sk-fixture-admin'}, body: {user_id: fixture-created, user_role: internal_user, auto_create_key: false}}
upstream: []
database: {seed: {dashboard: {logs: []}, users: [{user_id: fixture-admin}, {user_id: fixture-internal}], keys: [{token: sk-fixture-admin, user_id: fixture-admin, role: proxy_admin}, {token: sk-fixture-internal, user_id: fixture-internal, role: internal_user}]}}
compare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}
";

const HASHED_INTERNAL_CASE: &str = "name: hashed-internal-key-denied
config: 'general_settings: {master_key: oracle-master}'
env: {}
request: {method: POST, path: /user/new, headers: {Authorization: 'Bearer sk-fixture-internal'}, body: {user_id: fixture-denied, user_role: internal_user, auto_create_key: false}}
upstream: []
database: {seed: {dashboard: {logs: []}, users: [{user_id: fixture-admin}, {user_id: fixture-internal}], keys: [{token: sk-fixture-admin, user_id: fixture-admin, role: proxy_admin}, {token: sk-fixture-internal, user_id: fixture-internal, role: internal_user}]}}
compare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}
";

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
        b"name: disabled-schema-update
config: 'general_settings: {master_key: oracle-master}'
env: {}
request: {method: GET, path: /health/liveliness, headers: {}, body: null}
upstream: []
database: {seed: {}}
compare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}
",
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

#[derive(Clone)]
struct ReplayCall {
    model: String,
    data: Object,
    streamed: bool,
}

struct ReplayGeneration {
    answer: Mutex<Option<Result<GenerationAnswer, Box<ProxyException>>>>,
    calls: Mutex<Vec<ReplayCall>>,
}

fn noted_generation(
    fake: &ReplayGeneration,
    call: &GenerateContentCall<'_>,
    streamed: bool,
) -> Result<GenerationAnswer, Box<ProxyException>> {
    fake.calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(ReplayCall {
            model: call.model.to_owned(),
            data: call.data.clone(),
            streamed,
        });
    fake.answer
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .unwrap_or_else(|| Err(Box::new(ProxyException::new(ProxyExceptionInit::default()))))
}

#[async_trait]
impl GenerateContentDispatch for ReplayGeneration {
    async fn generate(
        &self,
        call: GenerateContentCall<'_>,
    ) -> Result<GenerationAnswer, Box<ProxyException>> {
        noted_generation(self, &call, false)
    }

    async fn stream(
        &self,
        call: GenerateContentCall<'_>,
    ) -> Result<GenerationAnswer, Box<ProxyException>> {
        noted_generation(self, &call, true)
    }
}

struct ReplayTranslator {
    calls: Mutex<Vec<(String, Json)>>,
}

impl TranslateContents for ReplayTranslator {
    fn messages(
        &self,
        model: &str,
        contents: &Json,
    ) -> Result<Vec<ChatMessage>, Box<ProxyException>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((model.to_owned(), contents.clone()));
        Ok(vec![marker_message()])
    }
}

// the one message the replay's adapter answers, so the count call's messages prove the translation result is wired through
fn marker_message() -> ChatMessage {
    ChatMessage {
        role: ChatRole::User,
        content: Some(MessageContent::Text("translated contents".to_owned())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    }
}

struct ReplayCounter {
    reply: Mutex<Option<TokenCountResponse>>,
    calls: Mutex<Vec<(TokenCountRequest, bool)>>,
}

#[async_trait]
impl TokenCounter for ReplayCounter {
    async fn count_tokens(
        &self,
        request: &TokenCountRequest,
        call_endpoint: bool,
    ) -> Result<Option<TokenCountResponse>, Box<ProxyException>> {
        let reply = self
            .reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((request.clone(), call_endpoint));
        Ok(reply)
    }
}

// one genai corpus seed drives the route the path names: the scripted pipeline answers go through the same three ports B8774 wires
async fn replay_genai_case(case: &HttpCase, counted: Counted) {
    let mut replay = scripted_replay(case, counted);
    route_genai_request(case, &mut replay).await;
    if !carries_credential(case) {
        assert_refused(&replay);
        return;
    }
    if replay.failed {
        assert_provider_error(&replay);
        return;
    }
    match replay.operation.as_str() {
        "generateContent" => assert_generated(&replay),
        "streamGenerateContent" => assert_streamed(&replay),
        "countTokens" => assert_counted(&replay, counted),
        _ => panic!("an unknown genai operation reached the replay"),
    }
}

struct GenaiReplay {
    model: String,
    operation: String,
    upstream: Option<UpstreamAnswer>,
    body_model: Option<String>,
    failed: bool,
    contents: Json,
    generation: Arc<ReplayGeneration>,
    translator: Arc<ReplayTranslator>,
    counter: Arc<ReplayCounter>,
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

fn scripted_replay(case: &HttpCase, counted: Counted) -> GenaiReplay {
    let upstream = case.upstream.first().cloned();
    let (model, operation) = path_model_operation(&case.request.path);
    GenaiReplay {
        model: model.to_owned(),
        operation: operation.to_owned(),
        failed: upstream.as_ref().is_some_and(failed_answer),
        body_model: case
            .request
            .body
            .get("model")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        generation: Arc::new(ReplayGeneration {
            answer: Mutex::new(generation_answer(operation, upstream.as_ref())),
            calls: Mutex::new(Vec::new()),
        }),
        translator: Arc::new(ReplayTranslator {
            calls: Mutex::new(Vec::new()),
        }),
        counter: Arc::new(ReplayCounter {
            reply: Mutex::new(counter_reply(operation, upstream.as_ref(), counted)),
            calls: Mutex::new(Vec::new()),
        }),
        upstream,
        contents: seed_contents(case),
        status: StatusCode::OK,
        headers: HeaderMap::new(),
        body: Bytes::new(),
    }
}

fn seed_contents(case: &HttpCase) -> Json {
    serde_json::from_value::<Json>(
        case.request
            .body
            .get("contents")
            .cloned()
            .expect("the seed names contents"),
    )
    .expect("the contents are JSON")
}

fn path_model_operation(path: &str) -> (&str, &str) {
    let (model_path, operation) = path
        .rsplit_once(':')
        .expect("the path carries an operation suffix");
    let model = model_path
        .strip_prefix("/v1beta/models/")
        .or_else(|| model_path.strip_prefix("/models/"))
        .expect("the path names a model");
    (model, operation)
}

// a scripted answer outside 2xx is the exception the pipeline raises rather than a body it forwards
fn failed_answer(upstream: &UpstreamAnswer) -> bool {
    !(200..300).contains(&upstream.status)
}

fn generation_answer(
    operation: &str,
    upstream: Option<&UpstreamAnswer>,
) -> Option<Result<GenerationAnswer, Box<ProxyException>>> {
    let answer = upstream?;
    let headers = answer_headers(&answer.headers);
    if failed_answer(answer) {
        return Some(Err(Box::new(provider_exception(answer))));
    }
    match operation {
        "generateContent" => Some(Ok(GenerationAnswer {
            headers,
            body: GenerationBody::Json(json_answer(answer)),
        })),
        "streamGenerateContent" => Some(Ok(streamed_answer(headers, answer))),
        _ => None,
    }
}

fn provider_exception(upstream: &UpstreamAnswer) -> ProxyException {
    let body = upstream.body.clone().expect("a provider error body");
    ProxyException::new(ProxyExceptionInit {
        message: error_field(&body, "message"),
        error_type: error_field(&body, "type"),
        param: body
            .get("param")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        code: ExceptionCode::Status(i64::from(upstream.status)),
        headers: upstream.headers.clone(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

fn error_field(body: &serde_json::Value, name: &str) -> String {
    body.get(name)
        .and_then(serde_json::Value::as_str)
        .expect("the provider error carries its message and type")
        .to_owned()
}

fn json_answer(upstream: &UpstreamAnswer) -> Json {
    serde_json::from_value(upstream.body.clone().expect("a JSON answer"))
        .expect("the answer is JSON")
}

fn streamed_answer(headers: HeaderMap, upstream: &UpstreamAnswer) -> GenerationAnswer {
    let events = upstream.events.as_ref().expect("SSE events").clone();
    GenerationAnswer {
        headers,
        body: GenerationBody::Events(Box::pin(stream::iter(
            events
                .into_iter()
                .map(|event| Ok::<Vec<u8>, GenAiOutputError>(event.into_bytes())),
        ))),
    }
}

fn counter_reply(
    operation: &str,
    upstream: Option<&UpstreamAnswer>,
    counted: Counted,
) -> Option<TokenCountResponse> {
    if operation != "countTokens" {
        return None;
    }
    match counted {
        Counted::Original => Some(counted_response(upstream?)),
        Counted::Total(total) => Some(TokenCountResponse {
            total_tokens: total,
            original_response: None,
        }),
        Counted::None => None,
    }
}

fn counted_response(upstream: &UpstreamAnswer) -> TokenCountResponse {
    let original = serde_json::from_value::<TokenCountOriginal>(
        upstream.body.clone().expect("a provider count reply"),
    )
    .expect("the provider count reply decodes");
    TokenCountResponse {
        total_tokens: 0,
        original_response: Some(original),
    }
}

fn answer_headers(headers: &BTreeMap<String, String>) -> HeaderMap {
    let mut mapped = HeaderMap::new();
    for (name, value) in headers {
        mapped.insert(
            HeaderName::try_from(name.as_str()).expect("an upstream header name"),
            HeaderValue::from_str(value).expect("an upstream header value"),
        );
    }
    mapped
}

async fn route_genai_request(case: &HttpCase, replay: &mut GenaiReplay) {
    let generation: Arc<dyn GenerateContentDispatch> =
        Arc::<ReplayGeneration>::clone(&replay.generation);
    let translator: Arc<dyn TranslateContents> = Arc::<ReplayTranslator>::clone(&replay.translator);
    let counter: Arc<dyn TokenCounter> = Arc::<ReplayCounter>::clone(&replay.counter);
    let router = routes(GenaiRoutesState {
        generation,
        translator,
        token_counter: counter,
        include_call_id_in_error_body: false,
    });
    let answered = router
        .oneshot(genai_request(case))
        .await
        .expect("the router answers");
    replay.status = answered.status();
    replay.headers = answered.headers().clone();
    replay.body = to_bytes(answered.into_body(), usize::MAX)
        .await
        .expect("the response body reads");
}

fn genai_request(case: &HttpCase) -> Request<Body> {
    let mut builder = Request::builder()
        .method(case.request.method.as_str())
        .uri(case.request.path.as_str());
    for (name, value) in &case.request.headers {
        builder = builder.header(name, value);
    }
    if carries_credential(case) {
        builder = builder.extension(auth_context());
    }
    builder
        .body(Body::from(
            serde_json::to_vec(&case.request.body).expect("the request body serializes"),
        ))
        .expect("the request builds")
}

// B003's key auth inserts the AuthContext for a request that carries a credential, and a seed without one reaches the route as an unauthenticated request
fn carries_credential(case: &HttpCase) -> bool {
    case.request
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("authorization"))
}

fn auth_context() -> AuthContext {
    let principal = Principal {
        principal_type: PrincipalType::ServiceAccount,
        subject: "oracle".to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: None,
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles: Vec::new(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey(None),
    };
    AuthContext { principal }
}

fn single_call<T: Clone>(calls: &Mutex<Vec<T>>, what: &str) -> T {
    let recorded = calls.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(recorded.len(), 1, "one {what}");
    recorded.first().cloned().expect("one recorded call")
}

fn response_json(replay: &GenaiReplay) -> serde_json::Value {
    serde_json::from_slice(&replay.body).expect("a JSON answer")
}

fn scripted_answer(replay: &GenaiReplay) -> &UpstreamAnswer {
    replay.upstream.as_ref().expect("a provider answer")
}

fn call_count<T>(calls: &Mutex<Vec<T>>) -> usize {
    calls.lock().unwrap_or_else(PoisonError::into_inner).len()
}

fn port_calls(replay: &GenaiReplay) -> usize {
    call_count(&replay.generation.calls)
        .saturating_add(call_count(&replay.translator.calls))
        .saturating_add(call_count(&replay.counter.calls))
}

fn assert_generated(replay: &GenaiReplay) {
    let upstream = scripted_answer(replay);
    assert_eq!(replay.status, StatusCode::OK);
    assert_eq!(
        response_json(replay),
        upstream.body.clone().expect("the scripted body")
    );
    assert_eq!(
        response_header(&replay.headers, "content-type"),
        Some("application/json")
    );
    assert_pipeline_headers(replay, upstream);
    let call = single_call(&replay.generation.calls, "generation call");
    assert_eq!(call.model, replay.model);
    assert!(!call.streamed);
    let model = replay.body_model.as_deref().unwrap_or(&replay.model);
    assert_eq!(call.data.get("model"), Some(&Json::Text(model.to_owned())));
    assert_eq!(call.data.get("contents"), Some(&replay.contents));
}

// every x-litellm-* header the pipeline answers with must reach the client with the same value
fn assert_pipeline_headers(replay: &GenaiReplay, upstream: &UpstreamAnswer) {
    let mut checked = 0_usize;
    for (name, value) in &upstream.headers {
        if name.starts_with("x-litellm-") {
            assert_eq!(
                response_header(&replay.headers, name),
                Some(value.as_str()),
                "{name}"
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "the seed scripts an x-litellm-* header");
}

// on a provider error the client reads the exception's status and the error envelope openai_exception_handler sends
fn assert_provider_error(replay: &GenaiReplay) {
    let upstream = scripted_answer(replay);
    assert_eq!(
        replay.status,
        StatusCode::from_u16(upstream.status).expect("the scripted status")
    );
    let body = upstream.body.clone().expect("a provider error body");
    assert_eq!(
        response_json(replay),
        serde_json::json!({"error": {
            "message": error_field(&body, "message"),
            "type": error_field(&body, "type"),
            "param": body.get("param").and_then(serde_json::Value::as_str),
            "code": upstream.status.to_string(),
        }})
    );
    assert_pipeline_headers(replay, upstream);
}

fn assert_refused(replay: &GenaiReplay) {
    assert_eq!(replay.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(port_calls(replay), 0);
}

fn assert_streamed(replay: &GenaiReplay) {
    let upstream = scripted_answer(replay);
    assert_eq!(replay.status, StatusCode::OK);
    assert_eq!(
        response_header(&replay.headers, "content-type"),
        Some("text/event-stream")
    );
    assert_eq!(
        response_header(&replay.headers, "cache-control"),
        Some("no-cache")
    );
    assert_eq!(
        response_header(&replay.headers, "x-accel-buffering"),
        Some("no")
    );
    let expected = upstream.events.as_ref().expect("SSE events").concat();
    assert_eq!(
        String::from_utf8(replay.body.to_vec()).expect("SSE bytes are UTF-8"),
        expected
    );
    assert_pipeline_headers(replay, upstream);
    let call = single_call(&replay.generation.calls, "stream call");
    assert_eq!(call.model, replay.model);
    assert!(call.streamed);
    assert_eq!(call.data.get("stream"), Some(&Json::Bool(true)));
    assert_eq!(
        call.data.get("_litellm_skip_openai_stream_done"),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        call.data.get("_litellm_raw_sse_stream"),
        Some(&Json::Bool(true))
    );
}

fn assert_counted(replay: &GenaiReplay, counted: Counted) {
    assert_eq!(replay.status, StatusCode::OK);
    assert_eq!(response_json(replay), expected_count(replay, counted));
    let translation = single_call(&replay.translator.calls, "translation call");
    assert_eq!(translation.0, replay.model);
    assert_eq!(translation.1, replay.contents);
    let count = single_call(&replay.counter.calls, "count call");
    assert_eq!(count.0.model, replay.model);
    assert_eq!(count.0.contents, replay.contents);
    assert_eq!(count.0.messages, vec![marker_message()]);
    assert!(count.1);
}

// TokenCountDetailsResponse for the count answer the seed scripts: the provider reply, its bare total, or the zero default
fn expected_count(replay: &GenaiReplay, counted: Counted) -> serde_json::Value {
    match counted {
        Counted::Original => scripted_answer(replay)
            .body
            .clone()
            .expect("the provider count body"),
        Counted::Total(total) => {
            serde_json::json!({"totalTokens": total, "promptTokensDetails": []})
        }
        Counted::None => serde_json::json!({"totalTokens": 0, "promptTokensDetails": []}),
    }
}

fn response_header<'headers>(headers: &'headers HeaderMap, name: &str) -> Option<&'headers str> {
    headers.get(name)?.to_str().ok()
}
