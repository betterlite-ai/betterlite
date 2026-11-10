#![cfg(test)]

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};

use super::reply::{MAX_REPLY_BYTES, OllamaMetadataError, decode_ollama_model_reply};
use super::{
    DEFAULT_API_BASE, LiteLlmGlobals, OllamaModelInfo, OllamaSettings, OllamaTransport,
    RuntimeModelInfo, ShowBody, ShowRequest, ShowResponse, strip_ollama_model_prefix,
};
use crate::catalog::{ModelCatalog, UnknownFields};

#[derive(Debug, thiserror::Error)]
#[error("fixture transport failure")]
struct FixtureFailure;

#[derive(Clone, Debug)]
enum FixtureOutcome {
    Reply { status: u16, body: Vec<u8> },
    Failure,
}

#[derive(Clone, Debug)]
struct Recorded {
    url: String,
    authorization: Option<SecretString>,
    name: String,
}

#[derive(Debug)]
struct FixtureTransport {
    outcome: FixtureOutcome,
    recorded: Mutex<Vec<Recorded>>,
}

impl FixtureTransport {
    fn replying(status: u16, body: &str) -> Self {
        Self {
            outcome: FixtureOutcome::Reply {
                status,
                body: body.as_bytes().to_vec(),
            },
            recorded: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            outcome: FixtureOutcome::Failure,
            recorded: Mutex::new(Vec::new()),
        }
    }

    fn recorded(&self) -> Vec<Recorded> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn only_request(&self) -> Recorded {
        let mut recorded = self.recorded();
        assert_eq!(recorded.len(), 1);
        recorded.remove(0)
    }
}

fn authorization(recorded: &Recorded) -> Option<&str> {
    recorded
        .authorization
        .as_ref()
        .map(ExposeSecret::expose_secret)
}

impl OllamaTransport for FixtureTransport {
    type Error = FixtureFailure;

    fn post_show(&self, request: &ShowRequest) -> Result<ShowResponse, Self::Error> {
        let authorization = request
            .headers
            .iter()
            .find(|(name, _)| *name == "Authorization")
            .map(|(_, value)| SecretString::from(value.expose_secret().to_owned()));
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: request.url.clone(),
                authorization,
                name: request.body.name.clone(),
            });
        match &self.outcome {
            FixtureOutcome::Reply { status, body } => Ok(ShowResponse {
                status: *status,
                body: body.clone(),
            }),
            FixtureOutcome::Failure => Err(FixtureFailure),
        }
    }
}

struct Fixture<'context> {
    settings: &'context OllamaSettings,
    globals: &'context LiteLlmGlobals,
    catalog: &'context ModelCatalog,
}

fn settings(api_key: Option<&str>, api_base: Option<&str>) -> OllamaSettings {
    OllamaSettings {
        api_key: api_key.map(SecretString::from),
        api_base: api_base.map(str::to_owned),
    }
}

fn globals(
    api_key: Option<&str>,
    openai_key: Option<&str>,
    secret_manager: Option<&str>,
) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_key: api_key.map(SecretString::from),
        openai_key: openai_key.map(SecretString::from),
        secret_manager_api_key: secret_manager.map(SecretString::from),
    }
}

fn catalog(keys: &[&str]) -> ModelCatalog {
    let entries = keys
        .iter()
        .map(|key| format!("{key:?}:{{}}"))
        .collect::<Vec<_>>()
        .join(",");
    let bytes = format!("{{{entries}}}");
    ModelCatalog::parse(bytes.as_bytes(), UnknownFields::Reject).unwrap()
}

fn resolve(
    fixture: &Fixture<'_>,
    model: &str,
    api_base: Option<&str>,
    api_key: Option<&str>,
) -> (Recorded, RuntimeModelInfo) {
    let transport = FixtureTransport::replying(200, "{}");
    let resolver = OllamaModelInfo {
        settings: fixture.settings,
        globals: fixture.globals,
        catalog: fixture.catalog,
        transport: &transport,
    };
    let key = api_key.map(SecretString::from);
    let info = resolver
        .get_runtime_model_info(model, api_base, key.as_ref())
        .unwrap();
    (transport.only_request(), info)
}

fn default_info(key: &str) -> RuntimeModelInfo {
    RuntimeModelInfo {
        key: key.to_owned(),
        litellm_provider: "ollama".to_owned(),
        mode: "chat".to_owned(),
        supports_function_calling: None,
        input_cost_per_token: Decimal::ZERO,
        output_cost_per_token: Decimal::ZERO,
        max_tokens: None,
        max_input_tokens: None,
        max_output_tokens: None,
    }
}

fn successful_info(key: &str, max_tokens: Option<i64>, supports_tools: bool) -> RuntimeModelInfo {
    RuntimeModelInfo {
        key: key.to_owned(),
        litellm_provider: "ollama".to_owned(),
        mode: "chat".to_owned(),
        supports_function_calling: Some(supports_tools),
        input_cost_per_token: Decimal::ZERO,
        output_cost_per_token: Decimal::ZERO,
        max_tokens,
        max_input_tokens: max_tokens,
        max_output_tokens: max_tokens,
    }
}

const BASE_CASES: [(Option<&str>, &str); 15] = [
    (Some("http://explicit:1"), "http://explicit:1"),
    (None, "http://env:2"),
    (Some(""), "http://env:2"),
    (Some("http://host:3/"), "http://host:3"),
    (Some("http://host:3///"), "http://host:3"),
    (Some("http://host:3/api/generate"), "http://host:3"),
    (Some("http://host:3/api/chat"), "http://host:3"),
    (Some("http://host:3/api/embed"), "http://host:3"),
    (Some("http://host:3/api/embeddings"), "http://host:3"),
    (Some("http://host:3/api/show"), "http://host:3"),
    (Some("http://host:3/api/tags"), "http://host:3"),
    (Some("http://host:3/api/tags/"), "http://host:3"),
    (Some("http://host:3/api/tags//"), "http://host:3"),
    (Some("http://host:3/api/tags/x"), "http://host:3/api/tags/x"),
    (
        Some("http://host:3/prefix/api/generate/"),
        "http://host:3/prefix",
    ),
];

const DEFAULT_BASES: [(Option<&str>, &str); 3] = [
    (None, DEFAULT_API_BASE),
    (Some(""), DEFAULT_API_BASE),
    (Some("///"), ""),
];

fn base_selection() {
    let empty_catalog = catalog(&[]);
    let nothing = globals(None, None, None);
    let with_env = settings(None, Some("http://env:2"));
    let without_env = settings(None, None);
    let cases = BASE_CASES;
    let fixture = Fixture {
        settings: &with_env,
        globals: &nothing,
        catalog: &empty_catalog,
    };
    for (explicit_base, expected_base) in cases {
        let (recorded, _) = resolve(&fixture, "llama3", explicit_base, None);
        assert_eq!(recorded.url, format!("{expected_base}/api/show"));
        assert!(authorization(&recorded).is_none());
    }
    let fallback = Fixture {
        settings: &without_env,
        globals: &nothing,
        catalog: &empty_catalog,
    };
    let defaults = DEFAULT_BASES;
    for (explicit_base, expected_base) in defaults {
        let (recorded, _) = resolve(&fallback, "llama3", explicit_base, None);
        assert_eq!(recorded.url, format!("{expected_base}/api/show"));
    }
}

fn key_precedence_environment() {
    let empty_catalog = catalog(&[]);
    let keys = settings(Some("env-key"), None);
    let global_keys = globals(Some("global-key"), Some("openai-key"), Some("manager-key"));
    let fixture = Fixture {
        settings: &keys,
        globals: &global_keys,
        catalog: &empty_catalog,
    };
    let (explicit, _) = resolve(&fixture, "llama3", None, Some("explicit-key"));
    assert_eq!(authorization(&explicit), Some("Bearer explicit-key"));
    let (environment, _) = resolve(&fixture, "llama3", None, None);
    assert_eq!(authorization(&environment), Some("Bearer env-key"));
    let (empty_explicit, _) = resolve(&fixture, "llama3", None, Some(""));
    assert_eq!(authorization(&empty_explicit), Some("Bearer env-key"));
}

fn key_precedence_globals() {
    let empty_catalog = catalog(&[]);
    let no_settings = settings(None, None);
    let global_only = globals(Some("global-key"), Some("openai-key"), Some("manager-key"));
    let global_fixture = Fixture {
        settings: &no_settings,
        globals: &global_only,
        catalog: &empty_catalog,
    };
    let (global, _) = resolve(&global_fixture, "llama3", None, None);
    assert_eq!(authorization(&global), Some("Bearer global-key"));

    let openai_only = globals(None, Some("openai-key"), Some("manager-key"));
    let openai_fixture = Fixture {
        settings: &no_settings,
        globals: &openai_only,
        catalog: &empty_catalog,
    };
    let (openai, _) = resolve(&openai_fixture, "llama3", None, None);
    assert_eq!(authorization(&openai), Some("Bearer openai-key"));

    let managed_only = globals(None, None, Some("manager-key"));
    let managed_fixture = Fixture {
        settings: &no_settings,
        globals: &managed_only,
        catalog: &empty_catalog,
    };
    let (managed, _) = resolve(&managed_fixture, "llama3", None, None);
    assert_eq!(authorization(&managed), Some("Bearer manager-key"));
}

fn key_precedence() {
    key_precedence_environment();
    key_precedence_globals();
}

fn ambient_auth_suppression() {
    let empty_catalog = catalog(&[]);
    let keys = settings(Some("env-key"), None);
    let global_keys = globals(Some("global-key"), Some("openai-key"), Some("manager-key"));
    let fixture = Fixture {
        settings: &keys,
        globals: &global_keys,
        catalog: &empty_catalog,
    };
    let (supplied_base, _) = resolve(&fixture, "llama3", Some("http://explicit:1"), None);
    assert!(authorization(&supplied_base).is_none());
    let (supplied_empty_base, _) = resolve(&fixture, "llama3", Some(""), None);
    assert!(authorization(&supplied_empty_base).is_none());
    assert_eq!(
        supplied_empty_base.url,
        format!("{DEFAULT_API_BASE}/api/show")
    );
    let (with_key, _) = resolve(
        &fixture,
        "llama3",
        Some("http://explicit:1"),
        Some("explicit-key"),
    );
    assert_eq!(authorization(&with_key), Some("Bearer explicit-key"));
    let (empty_base_with_key, _) = resolve(&fixture, "llama3", Some(""), Some("explicit-key"));
    assert_eq!(
        authorization(&empty_base_with_key),
        Some("Bearer explicit-key")
    );
    let (empty_base_empty_key, _) = resolve(&fixture, "llama3", Some(""), Some(""));
    assert!(authorization(&empty_base_empty_key).is_none());
    let (stripped, _) = resolve(&fixture, "ollama/llama3", None, None);
    assert_eq!(stripped.name, "llama3");
    let (chat_stripped, _) = resolve(&fixture, "ollama_chat/llama3", None, None);
    assert_eq!(chat_stripped.name, "llama3");
}

fn configured_origin_auth() {
    base_selection();
    key_precedence();
    ambient_auth_suppression();
}

fn unmapped_model_queries_http() {
    let empty_settings = settings(None, None);
    let nothing = globals(None, None, None);
    let empty_catalog = catalog(&[]);
    let transport = FixtureTransport::replying(200, "{}");
    let fixture = Fixture {
        settings: &empty_settings,
        globals: &nothing,
        catalog: &empty_catalog,
    };
    let resolver = OllamaModelInfo {
        settings: fixture.settings,
        globals: fixture.globals,
        catalog: fixture.catalog,
        transport: &transport,
    };
    let info = resolver
        .get_model_info("missing-model", None, None)
        .unwrap()
        .unwrap();
    assert_eq!(info, successful_info("missing-model", None, false));
    assert_eq!(transport.only_request().name, "missing-model");
}

const STATIC_CATALOG: [&str; 5] = [
    "ollama/llama2",
    "Ollama/Mixtral",
    "plain-model",
    "ollama_chat/codellama",
    "namespace/model:tag",
];

const STATIC_MODELS: [&str; 12] = [
    "llama2",
    "ollama/llama2",
    "LLAMA2",
    "ollama/LLAMA2",
    "mixtral",
    "OLLAMA/MIXTRAL",
    "ollama_chat/llama2",
    "plain-model",
    "ollama_chat/plain-model",
    "codellama",
    "ollama/codellama",
    "namespace/model:tag",
];

fn static_catalog_first() {
    let empty_settings = settings(None, None);
    let nothing = globals(None, None, None);
    let catalog = catalog(&STATIC_CATALOG);
    let fixture = Fixture {
        settings: &empty_settings,
        globals: &nothing,
        catalog: &catalog,
    };
    let transport = FixtureTransport::replying(200, "{}");
    let resolver = OllamaModelInfo {
        settings: fixture.settings,
        globals: fixture.globals,
        catalog: fixture.catalog,
        transport: &transport,
    };
    for model in STATIC_MODELS {
        assert!(
            resolver
                .get_model_info(model, None, None)
                .unwrap()
                .is_none()
        );
    }
    assert!(transport.recorded().is_empty());
    unmapped_model_queries_http();
}

fn query_request_shape() {
    let keyed = settings(Some("secret-key"), Some("http://host:9/api/chat"));
    let nothing = globals(None, None, None);
    let empty_catalog = catalog(&[]);
    let fixture = Fixture {
        settings: &keyed,
        globals: &nothing,
        catalog: &empty_catalog,
    };
    let (recorded, info) = resolve(&fixture, "ollama/llama3:8b", None, None);
    assert_eq!(recorded.url, "http://host:9/api/show");
    assert_eq!(recorded.name, "llama3:8b");
    assert_eq!(authorization(&recorded), Some("Bearer secret-key"));
    assert_eq!(info, successful_info("llama3:8b", None, false));
}

fn query_status_fallbacks() {
    let empty_settings = settings(None, None);
    let nothing = globals(None, None, None);
    let empty_catalog = catalog(&[]);
    let fixture = Fixture {
        settings: &empty_settings,
        globals: &nothing,
        catalog: &empty_catalog,
    };
    let fallback_body = "{\"model_info\":{\"llama.context_length\":2048}}";
    for status in [199_u16, 300, 304, 399, 400, 404, 500] {
        let transport = FixtureTransport::replying(status, fallback_body);
        let resolver = OllamaModelInfo {
            settings: fixture.settings,
            globals: fixture.globals,
            catalog: fixture.catalog,
            transport: &transport,
        };
        let info = resolver
            .get_runtime_model_info("ollama/llama3:8b", None, None)
            .unwrap();
        assert_eq!(info, default_info("llama3:8b"));
        let recorded = transport.only_request();
        assert_eq!(recorded.url, format!("{DEFAULT_API_BASE}/api/show"));
        assert_eq!(recorded.name, "llama3:8b");
        assert!(authorization(&recorded).is_none());
    }
    let failing = FixtureTransport::failing();
    let resolver = OllamaModelInfo {
        settings: fixture.settings,
        globals: fixture.globals,
        catalog: fixture.catalog,
        transport: &failing,
    };
    let info = resolver
        .get_runtime_model_info("llama3", None, None)
        .unwrap();
    assert_eq!(info, default_info("llama3"));
}

fn query_success_boundary() {
    let empty_settings = settings(None, None);
    let nothing = globals(None, None, None);
    let empty_catalog = catalog(&[]);
    let transport = FixtureTransport::replying(
        299,
        "{\"template\":\"tools\",\"model_info\":{\"llama.context_length\":4096}}",
    );
    let resolver = OllamaModelInfo {
        settings: &empty_settings,
        globals: &nothing,
        catalog: &empty_catalog,
        transport: &transport,
    };
    let info = resolver
        .get_runtime_model_info("llama3", None, None)
        .unwrap();
    assert_eq!(info, successful_info("llama3", Some(4096), true));
}

fn runtime_model_query() {
    query_request_shape();
    query_status_fallbacks();
    query_success_boundary();
}

const FIELD_LIMIT_CASES: [(&str, Option<i64>, bool); 6] = [
    (
        "{\"template\":\"{{ .Tools }}\",\"model_info\":{\"z.context_length\":8192,\"a.context_length\":4096}}",
        Some(8192),
        true,
    ),
    (
        "{\"model_info\":{\"llama.context_length\":4096}}",
        Some(4096),
        false,
    ),
    (
        "{\"model_info\":{\"llama.context_length\":null,\"other.context_length\":7}}",
        None,
        false,
    ),
    (
        "{\"model_info\":{\"llama.embedding_length\":4096}}",
        None,
        false,
    ),
    (
        "{\"model_info\":{\"weird_context_length_extra\":12}}",
        Some(12),
        false,
    ),
    (
        "{\"model_info\":{\"llama.context_length\":8192,\"extra\":\"x\"}}",
        Some(8192),
        false,
    ),
];

const FIELD_TEMPLATE_CASES: [(&str, Option<i64>, bool); 7] = [
    (
        "{\"template\":\"TOOLS\",\"model_info\":{\"llama.context_length\":4096}}",
        Some(4096),
        true,
    ),
    (
        "{\"template\":\"tool\",\"model_info\":{\"llama.context_length\":5}}",
        Some(5),
        false,
    ),
    ("{\"template\":\"no tools here\"}", None, true),
    ("{\"model_info\":{}}", None, false),
    ("{}", None, false),
    ("{\"template\":null}", None, false),
    ("{\"template\":\"\"}", None, false),
];

fn field_cases() {
    let empty_settings = settings(None, None);
    let nothing = globals(None, None, None);
    let empty_catalog = catalog(&[]);
    let fixture = Fixture {
        settings: &empty_settings,
        globals: &nothing,
        catalog: &empty_catalog,
    };
    for (body, max_tokens, supports_tools) in
        FIELD_LIMIT_CASES.into_iter().chain(FIELD_TEMPLATE_CASES)
    {
        let transport = FixtureTransport::replying(200, body);
        let resolver = OllamaModelInfo {
            settings: fixture.settings,
            globals: fixture.globals,
            catalog: fixture.catalog,
            transport: &transport,
        };
        let info = resolver
            .get_runtime_model_info("ollama/qwen", None, None)
            .unwrap();
        assert_eq!(
            info,
            successful_info("qwen", max_tokens, supports_tools),
            "{body}"
        );
    }
}

const DECODE_FAILURES: [&str; 11] = [
    "",
    "{\"model_info\":",
    "{\"template\":\"tools\",\"model_info\":{\"llama.context_length\":819",
    "[]",
    "\"text\"",
    "null",
    "{\"model_info\":null}",
    "{\"model_info\":[]}",
    "{\"template\":5}",
    "{\"model_info\":{\"llama.context_length\":\"8192\"}}",
    "{\"model_info\":{\"llama.context_length\":8192.5}}",
];

fn field_decode_failures() {
    let empty_settings = settings(None, None);
    let nothing = globals(None, None, None);
    let empty_catalog = catalog(&[]);
    for body in DECODE_FAILURES {
        let transport = FixtureTransport::replying(200, body);
        let resolver = OllamaModelInfo {
            settings: &empty_settings,
            globals: &nothing,
            catalog: &empty_catalog,
            transport: &transport,
        };
        assert!(
            resolver.get_runtime_model_info("qwen", None, None).is_err(),
            "{body}"
        );
    }
    let oversized = vec![b' '; MAX_REPLY_BYTES + 1];
    assert!(matches!(
        decode_ollama_model_reply(&oversized),
        Err(OllamaMetadataError::ReplyTooLarge { .. })
    ));
    assert!(matches!(
        decode_ollama_model_reply(b"{\"model_info\":{\"c.context_length\":\"x\"}}"),
        Err(OllamaMetadataError::ReplyField { .. })
    ));
    assert!(matches!(
        decode_ollama_model_reply(b"{\"template\":5}"),
        Err(OllamaMetadataError::Reply { .. })
    ));
}

fn reply_size_limit() {
    const LIMIT: usize = 16 * 1024 * 1024;
    assert_eq!(MAX_REPLY_BYTES, LIMIT);
    let at_limit = vec![b' '; LIMIT];
    assert!(matches!(
        decode_ollama_model_reply(&at_limit),
        Err(OllamaMetadataError::Reply { .. })
    ));
    let over_limit = vec![b' '; LIMIT + 1];
    assert!(matches!(
        decode_ollama_model_reply(&over_limit),
        Err(OllamaMetadataError::ReplyTooLarge { .. })
    ));
}

fn runtime_model_fields() {
    field_cases();
    field_decode_failures();
    reply_size_limit();
}

fn stripped_prefixes() {
    assert_eq!(strip_ollama_model_prefix("ollama/llama3"), "llama3");
    assert_eq!(strip_ollama_model_prefix("ollama_chat/llama3"), "llama3");
    assert_eq!(strip_ollama_model_prefix("llama3"), "llama3");
    assert_eq!(strip_ollama_model_prefix("ollama/"), "");
    assert_eq!(strip_ollama_model_prefix("ollama_chat/"), "");
    assert_eq!(strip_ollama_model_prefix("ollama/org/model"), "org/model");
    assert_eq!(strip_ollama_model_prefix("OLLAMA/llama3"), "OLLAMA/llama3");
    assert_eq!(strip_ollama_model_prefix("ollama_chat"), "ollama_chat");
}

#[rstest]
#[case::configured_origin_auth(0)]
#[case::static_catalog_first(1)]
#[case::runtime_model_query(2)]
#[case::runtime_model_fields(3)]
#[case::stripped_prefixes(4)]
fn ollama_model_metadata_cases(#[case] case: u8) {
    match case {
        0 => configured_origin_auth(),
        1 => static_catalog_first(),
        2 => runtime_model_query(),
        3 => runtime_model_fields(),
        _ => stripped_prefixes(),
    }
}

fn isolation_first_calls() {
    let first_settings = settings(Some("first-key"), Some("http://first:1"));
    let second_settings = settings(Some("second-key"), Some("http://second:2"));
    let nothing = globals(None, None, None);
    let first_catalog = catalog(&["ollama/shared"]);
    let second_catalog = catalog(&["ollama/second-only"]);
    let first_transport =
        FixtureTransport::replying(200, "{\"model_info\":{\"m.context_length\":1}}");
    let second_transport =
        FixtureTransport::replying(200, "{\"model_info\":{\"m.context_length\":2}}");
    let first = OllamaModelInfo {
        settings: &first_settings,
        globals: &nothing,
        catalog: &first_catalog,
        transport: &first_transport,
    };
    let second = OllamaModelInfo {
        settings: &second_settings,
        globals: &nothing,
        catalog: &second_catalog,
        transport: &second_transport,
    };
    let first_info = first.get_runtime_model_info("shared", None, None).unwrap();
    let second_info = second.get_runtime_model_info("shared", None, None).unwrap();
    assert_eq!(first_info.max_tokens, Some(1));
    assert_eq!(second_info.max_tokens, Some(2));
    let first_request = first_transport.only_request();
    let second_request = second_transport.only_request();
    assert_eq!(first_request.url, "http://first:1/api/show");
    assert_eq!(second_request.url, "http://second:2/api/show");
    assert_eq!(authorization(&first_request), Some("Bearer first-key"));
    assert_eq!(authorization(&second_request), Some("Bearer second-key"));
}

fn isolation_static_checks() {
    let first_settings = settings(Some("first-key"), Some("http://first:1"));
    let second_settings = settings(Some("second-key"), Some("http://second:2"));
    let nothing = globals(None, None, None);
    let first_catalog = catalog(&["ollama/shared"]);
    let second_catalog = catalog(&["ollama/second-only"]);
    let first_transport = FixtureTransport::replying(200, "{}");
    let second_transport = FixtureTransport::replying(200, "{}");
    let first = OllamaModelInfo {
        settings: &first_settings,
        globals: &nothing,
        catalog: &first_catalog,
        transport: &first_transport,
    };
    let second = OllamaModelInfo {
        settings: &second_settings,
        globals: &nothing,
        catalog: &second_catalog,
        transport: &second_transport,
    };
    assert!(
        first
            .get_model_info("second-only", None, None)
            .unwrap()
            .is_some()
    );
    assert!(
        second
            .get_model_info("second-only", None, None)
            .unwrap()
            .is_none()
    );
    assert!(authorization(&first_transport.only_request()).is_some());
    assert!(second_transport.recorded().is_empty());
}

#[test]
fn ollama_model_metadata_credential_identity_isolation() {
    isolation_first_calls();
    isolation_static_checks();
}

fn debug_request(recorded: &Recorded, key: &str) -> ShowRequest {
    ShowRequest {
        url: recorded.url.clone(),
        body: ShowBody {
            name: "llama3".to_owned(),
        },
        headers: vec![("Authorization", SecretString::from(format!("Bearer {key}")))],
    }
}

fn secret_safe_debug_outputs() {
    const FIXTURE_KEY: &str = "fixture-secret-key-value";
    const FIXTURE_MANAGER: &str = "fixture-manager-secret";
    let fixture_settings = settings(Some(FIXTURE_KEY), Some("http://host:1"));
    let fixture_globals = globals(None, None, Some(FIXTURE_MANAGER));
    let empty_catalog = catalog(&[]);
    let transport = FixtureTransport::failing();
    let resolver = OllamaModelInfo {
        settings: &fixture_settings,
        globals: &fixture_globals,
        catalog: &empty_catalog,
        transport: &transport,
    };
    let info = resolver
        .get_runtime_model_info("llama3", None, None)
        .unwrap();
    assert_eq!(info, default_info("llama3"));
    let recorded = transport.only_request();
    let expected_authorization = format!("Bearer {FIXTURE_KEY}");
    assert_eq!(
        authorization(&recorded),
        Some(expected_authorization.as_str())
    );
    let request = debug_request(&recorded, FIXTURE_KEY);
    let debugged = [
        format!("{fixture_settings:?}"),
        format!("{fixture_globals:?}"),
        format!("{resolver:?}"),
        format!("{request:?}"),
        format!("{info:?}"),
    ];
    for text in debugged {
        assert!(!text.contains(FIXTURE_KEY), "{text}");
        assert!(!text.contains(FIXTURE_MANAGER), "{text}");
    }
}

fn secret_safe_encode_errors() {
    const FIXTURE_KEY: &str = "fixture-secret-key-value";
    let settings = settings(Some(FIXTURE_KEY), None);
    let nothing = globals(None, None, None);
    let empty_catalog = catalog(&[]);
    let transport = FixtureTransport::replying(200, "not json");
    let resolver = OllamaModelInfo {
        settings: &settings,
        globals: &nothing,
        catalog: &empty_catalog,
        transport: &transport,
    };
    let first = resolver
        .get_runtime_model_info("llama3", None, None)
        .unwrap_err();
    let second =
        decode_ollama_model_reply(b"{\"model_info\":{\"c.context_length\":\"x\"}}").unwrap_err();
    for text in [format!("{first:?} {first}"), format!("{second:?} {second}")] {
        assert!(!text.contains(FIXTURE_KEY), "{text}");
    }
}

fn secret_safe_failure_paths() {
    const FIXTURE_KEY: &str = "fixture-secret-key-value";
    let fixture_settings = settings(Some(FIXTURE_KEY), Some("http://host:1"));
    let nothing = globals(None, None, None);
    let empty_catalog = catalog(&[]);
    let transport = FixtureTransport::replying(200, "{\"model_info\":null}");
    let resolver = OllamaModelInfo {
        settings: &fixture_settings,
        globals: &nothing,
        catalog: &empty_catalog,
        transport: &transport,
    };
    let error = resolver
        .get_runtime_model_info("llama3", None, None)
        .unwrap_err();
    let text = format!("{error:?} {error}");
    assert!(!text.contains(FIXTURE_KEY), "{text}");
}

#[test]
fn ollama_model_metadata_secret_safe_failure() {
    secret_safe_debug_outputs();
    secret_safe_encode_errors();
    secret_safe_failure_paths();
}

#[rstest]
fn replays_the_ollama_model_reply_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/ollama_model_reply");
    let mut entries = fs::read_dir(&directory)
        .expect("the ollama_model_reply corpus directory exists")
        .collect::<Result<Vec<_>, _>>()
        .expect("every corpus entry is readable");
    entries.sort_by_key(fs::DirEntry::file_name);
    assert!(entries.len() >= 4);
    for entry in entries {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .expect("a corpus file has a UTF-8 name");
        let body = fs::read(&path).expect("a corpus file is readable");
        let outcome = decode_ollama_model_reply(&body);
        assert_eq!(
            outcome.is_ok(),
            name.starts_with("valid_"),
            "{name}: {outcome:?}"
        );
    }
}
