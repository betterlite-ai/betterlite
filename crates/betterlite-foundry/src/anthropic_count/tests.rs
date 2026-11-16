use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_anthropic::count_tokens::counter::{TokenCountResponse, TokenCounter as _};
use betterlite_anthropic::count_tokens::handler::CountTokensTransport;
use betterlite_anthropic::count_tokens::wire::{CountResponse, CountTokensBody};
use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::anthropic_count_tokens::{DEFAULT_BETA_VERSION, Raw as BetaRaw};
use betterlite_config::sections::azure_credentials::{DeploymentSettings, Raw as CredentialRaw};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::anthropic_count::counter::{AzureAnthropicTokenCounter, CountTokensCall};
use crate::anthropic_count::endpoint::get_count_tokens_endpoint;
use crate::anthropic_count::error::ApiError;
use crate::anthropic_count::handler::{AzureAnthropicCountTokensHandler, CountTokensRequest};
use crate::anthropic_count::reply::decode_foundry_count_reply;
use crate::anthropic_messages::environment::{ParamsKey, TokenSource};

const MODEL: &str = "claude-sonnet-4-5";
const API_BASE: &str = "https://r.services.ai.azure.com";
const USER_MESSAGE: &str = r#"[{"role":"user","content":"hi"}]"#;
const ARGUMENT_KEY: &str = "argument-key";
const PARAMS_KEY: &str = "params-key";
const TOKENIZER_TYPE: &str = "azure_ai_anthropic_api";

#[derive(Clone, Copy)]
enum Planned {
    Endpoint,
    Auth,
    NativeResult,
    Selection,
    Failure,
    Warnings,
}

#[rstest]
#[case::count_tokens_endpoint(Planned::Endpoint)]
#[case::azure_auth_and_beta(Planned::Auth)]
#[case::native_count_result(Planned::NativeResult)]
#[case::optional_native_selection(Planned::Selection)]
#[case::count_failure_error_result(Planned::Failure)]
#[case::count_error_warnings(Planned::Warnings)]
fn anthropic_count_cases(#[case] planned: Planned) {
    match planned {
        Planned::Endpoint => count_tokens_endpoint(),
        Planned::Auth => azure_auth_and_beta(),
        Planned::NativeResult => native_count_result(),
        Planned::Selection => optional_native_selection(),
        Planned::Failure => count_failure_error_result(),
        Planned::Warnings => count_error_warnings(),
    }
}

fn count_tokens_endpoint() {
    let cases = [
        (
            "https://r.services.ai.azure.com",
            "https://r.services.ai.azure.com/anthropic/v1/messages/count_tokens",
        ),
        (
            "https://r.services.ai.azure.com/",
            "https://r.services.ai.azure.com/anthropic/v1/messages/count_tokens",
        ),
        (
            "https://r.services.ai.azure.com///",
            "https://r.services.ai.azure.com/anthropic/v1/messages/count_tokens",
        ),
        (
            "https://r.services.ai.azure.com/anthropic",
            "https://r.services.ai.azure.com/anthropic/v1/messages/count_tokens",
        ),
        (
            "https://r.services.ai.azure.com/anthropic/",
            "https://r.services.ai.azure.com/anthropic/v1/messages/count_tokens",
        ),
        (
            "https://r.services.ai.azure.com/anthropic/v1/messages/count_tokens",
            "https://r.services.ai.azure.com/anthropic/v1/messages/count_tokens/v1/messages/count_tokens",
        ),
        (
            "https://proxy.example/v1/messages?x=/anthropic",
            "https://proxy.example/v1/messages?x=/anthropic/v1/messages/count_tokens",
        ),
        (
            "https://r.services.ai.azure.com/Anthropic",
            "https://r.services.ai.azure.com/Anthropic/anthropic/v1/messages/count_tokens",
        ),
        ("", "/anthropic/v1/messages/count_tokens"),
    ];
    for (api_base, expected) in cases {
        assert_eq!(get_count_tokens_endpoint(api_base), expected, "{api_base}");
    }
}

fn azure_auth_and_beta() {
    let beta = beta_version(&BTreeMap::new());
    assert_eq!(beta, DEFAULT_BETA_VERSION);

    let plain = handle_headers(&Call::default());
    assert_eq!(plain["Content-Type"], "application/json");
    assert_eq!(plain["anthropic-version"], "2023-06-01");
    assert_eq!(plain["anthropic-beta"], beta.as_str());
    assert_eq!(plain["x-api-key"], ARGUMENT_KEY);
    assert_eq!(plain["api-key"], PARAMS_KEY);

    let absent = handle_headers(&Call {
        params: Params::Absent,
        ..Call::default()
    });
    assert_eq!(absent["x-api-key"], ARGUMENT_KEY);
    assert_eq!(absent["api-key"], ARGUMENT_KEY);

    let other = Call {
        params: Params::Key(SecretString::from("other-params-key")),
        ..Call::default()
    };
    let different = handle_headers(&other);
    assert_eq!(different["x-api-key"], ARGUMENT_KEY);
    assert_eq!(different["api-key"], "other-params-key");

    let configured = Call {
        params: Params::Null,
        configured_key: Some(SecretString::from("configured-key")),
        ..Call::default()
    };
    let nulled = handle_headers(&configured);
    assert_eq!(nulled["x-api-key"], ARGUMENT_KEY);
    assert_eq!(nulled["api-key"], "configured-key");

    let bearer = Call {
        params: Params::Null,
        token: Some("azure-token".to_owned()),
        ..Call::default()
    };
    let entranced = handle_headers(&bearer);
    assert_eq!(entranced["x-api-key"], ARGUMENT_KEY);
    assert_eq!(entranced["Authorization"], "Bearer azure-token");
    assert!(!entranced.contains_key("api-key"));

    let typed = Call {
        params: Params::Typed(Some(SecretString::from("typed-key"))),
        ..Call::default()
    };
    let typed_headers = handle_headers(&typed);
    assert_eq!(typed_headers["x-api-key"], ARGUMENT_KEY);
    assert_eq!(typed_headers["api-key"], "typed-key");

    let typed_empty = Call {
        params: Params::Typed(Some(SecretString::from(""))),
        ..Call::default()
    };
    let typed_empty_headers = handle_headers(&typed_empty);
    assert_eq!(typed_empty_headers["x-api-key"], ARGUMENT_KEY);
    assert_eq!(typed_empty_headers["api-key"], ARGUMENT_KEY);

    let empty_key_configured = Call {
        params: Params::Key(SecretString::from("")),
        configured_key: Some(SecretString::from("configured-key")),
        ..Call::default()
    };
    let empty_key_headers = handle_headers(&empty_key_configured);
    assert_eq!(empty_key_headers["x-api-key"], ARGUMENT_KEY);
    assert_eq!(empty_key_headers["api-key"], "configured-key");
}

fn native_count_result() {
    let success = run_counter(&Call::default());
    assert_eq!(success.selected, [true, false, false]);
    let result = success.result.expect("a token count");
    assert_eq!(result.total_tokens, 5);
    assert_eq!(result.request_model, MODEL);
    assert_eq!(result.model_used, MODEL);
    assert_eq!(result.tokenizer_type, TOKENIZER_TYPE);
    assert!(!result.error);
    assert_eq!(result.status_code, None);
    assert_eq!(
        result.original_response,
        Some(
            serde_json::from_str::<CountResponse>(r#"{"input_tokens":5}"#)
                .expect("valid fixture JSON")
        )
    );

    let missing = run_counter(&Call {
        reply: Some((200, "{}".to_owned())),
        ..Call::default()
    });
    assert_eq!(missing.result.expect("a token count").total_tokens, 0);

    let nulled = run_counter(&Call {
        reply: Some((200, "null".to_owned())),
        ..Call::default()
    });
    assert!(nulled.result.is_none());

    let empty = run_counter(&Call {
        messages: Some(Json::List(Vec::new())),
        ..Call::default()
    });
    assert!(empty.result.is_none());
    assert!(
        empty.seen.is_none(),
        "falsey messages never reach the transport"
    );

    let absent = run_counter(&Call {
        messages: None,
        ..Call::default()
    });
    assert!(absent.result.is_none());
    assert!(
        absent.seen.is_none(),
        "contents never stands in for messages"
    );

    for falsey_messages in [r#"null"#, r#"false"#, r#"0"#, r#""""#, r#"[]"#, r#"{}"#] {
        let skipped = run_counter(&Call {
            messages: Some(
                serde_json::from_str::<Json>(falsey_messages).expect("valid fixture JSON"),
            ),
            ..Call::default()
        });
        assert!(skipped.result.is_none(), "{falsey_messages}");
        assert!(skipped.seen.is_none(), "{falsey_messages}");
    }
}

fn optional_native_selection() {
    let from_environment = run_counter(&Call {
        params: Params::Absent,
        api_base: None,
        environment_key: Some(SecretString::from("environment-key")),
        environment_base: Some("https://env.services.ai.azure.com".to_owned()),
        ..Call::default()
    });
    let environment_seen = from_environment.seen.expect("a request");
    assert_eq!(
        environment_seen.url,
        "https://env.services.ai.azure.com/anthropic/v1/messages/count_tokens"
    );
    assert_eq!(environment_seen.headers["x-api-key"], "environment-key");

    let deployment = run_counter(&Call::default());
    let deployment_seen = deployment.seen.expect("a request");
    assert_eq!(
        deployment_seen.url,
        "https://r.services.ai.azure.com/anthropic/v1/messages/count_tokens"
    );
    assert_eq!(deployment_seen.headers["x-api-key"], PARAMS_KEY);

    let no_key = run_counter(&Call {
        params: Params::Absent,
        ..Call::default()
    });
    assert!(no_key.result.is_none());
    assert!(no_key.seen.is_none());
    assert_eq!(
        no_key.warnings,
        [Warning {
            subject: "No Azure AI API key found for token counting".to_owned(),
            status: None,
        }]
    );

    let no_base = run_counter(&Call {
        api_base: None,
        ..Call::default()
    });
    assert!(no_base.result.is_none());
    assert!(no_base.seen.is_none());
    assert_eq!(
        no_base.warnings,
        [Warning {
            subject: "No Azure AI API base found for token counting".to_owned(),
            status: None,
        }]
    );

    let empty = run_counter(&Call {
        params: Params::Key(SecretString::from("")),
        environment_key: Some(SecretString::from("environment-key")),
        ..Call::default()
    });
    let empty_seen = empty.seen.expect("a request");
    assert_eq!(empty_seen.headers["x-api-key"], "environment-key");

    let empty_environment_key = run_counter(&Call {
        params: Params::Absent,
        environment_key: Some(SecretString::from("")),
        ..Call::default()
    });
    assert!(empty_environment_key.result.is_none());
    assert!(empty_environment_key.seen.is_none());
    assert_eq!(
        empty_environment_key.warnings,
        [Warning {
            subject: "No Azure AI API key found for token counting".to_owned(),
            status: None,
        }]
    );

    let empty_deployment_base = run_counter(&Call {
        api_base: Some(String::new()),
        environment_base: Some("https://env.services.ai.azure.com".to_owned()),
        ..Call::default()
    });
    let empty_deployment_seen = empty_deployment_base.seen.expect("a request");
    assert_eq!(
        empty_deployment_seen.url,
        "https://env.services.ai.azure.com/anthropic/v1/messages/count_tokens"
    );

    let empty_environment_base = run_counter(&Call {
        api_base: None,
        environment_base: Some(String::new()),
        ..Call::default()
    });
    assert!(empty_environment_base.result.is_none());
    assert!(empty_environment_base.seen.is_none());
    assert_eq!(
        empty_environment_base.warnings,
        [Warning {
            subject: "No Azure AI API base found for token counting".to_owned(),
            status: None,
        }]
    );

    let credentials_only = run_counter(&Call {
        params: Params::Absent,
        token: Some("azure-token".to_owned()),
        ..Call::default()
    });
    assert!(
        credentials_only.result.is_none(),
        "Entra credentials never replace the key"
    );
    assert!(credentials_only.seen.is_none());

    let configured_only = run_counter(&Call {
        params: Params::Absent,
        configured_key: Some(SecretString::from("configured-key")),
        ..Call::default()
    });
    assert!(
        configured_only.result.is_none(),
        "the global key is not a counter fallback"
    );
}

fn count_failure_error_result() {
    counter_error_records();
    handler_failure_shapes();
}

fn counter_error_records() {
    let rate_limited = run_counter(&Call {
        reply: Some((429, "rate limited".to_owned())),
        ..Call::default()
    });
    let limited_record = rate_limited.result.expect("an error record");
    assert!(limited_record.error);
    assert_eq!(limited_record.total_tokens, 0);
    assert_eq!(limited_record.status_code, Some(429));
    assert_eq!(
        limited_record.error_message.as_deref(),
        Some("rate limited")
    );
    assert_eq!(limited_record.original_response, None);
    assert_eq!(limited_record.tokenizer_type, TOKENIZER_TYPE);

    let bad_request = run_counter(&Call {
        reply: Some((400, "{\"error\":\"bad\"}".to_owned())),
        ..Call::default()
    });
    let bad_record = bad_request.result.expect("an error record");
    assert_eq!(bad_record.status_code, Some(400));
    assert_eq!(
        bad_record.error_message.as_deref(),
        Some("{\"error\":\"bad\"}")
    );

    let created = run_counter(&Call {
        reply: Some((201, "{\"input_tokens\":5}".to_owned())),
        ..Call::default()
    });
    let created_record = created.result.expect("an error record");
    assert_eq!(created_record.status_code, Some(201));
    assert_eq!(
        created_record.error_message.as_deref(),
        Some("{\"input_tokens\":5}")
    );

    let not_json = run_counter(&Call {
        reply: Some((200, "not json".to_owned())),
        ..Call::default()
    });
    let decode_record = not_json.result.expect("an error record");
    assert_eq!(decode_record.status_code, Some(500));
    assert!(
        decode_record
            .error_message
            .as_deref()
            .is_some_and(|message| message.starts_with("CountTokens processing error: "))
    );

    let list = run_counter(&Call {
        reply: Some((200, "[]".to_owned())),
        ..Call::default()
    });
    let list_record = list.result.expect("an error record");
    assert_eq!(list_record.status_code, Some(500));
    assert!(
        list_record
            .error_message
            .as_deref()
            .is_some_and(|message| !message.starts_with("CountTokens processing error: "))
    );

    let null_tokens = run_counter(&Call {
        reply: Some((200, "{\"input_tokens\":null}".to_owned())),
        ..Call::default()
    });
    assert_eq!(
        null_tokens.result.expect("an error record").status_code,
        Some(500)
    );

    let failed_transport = run_counter(&Call {
        reply: None,
        ..Call::default()
    });
    assert_eq!(
        failed_transport
            .result
            .expect("an error record")
            .status_code,
        Some(500)
    );
}

fn handler_failure_shapes() {
    let sent = run_handle(&Call::default()).seen.expect("a request");
    assert_eq!(
        sent.body,
        serde_json::from_str::<Json>(
            r#"{"model":"claude-sonnet-4-5","messages":[{"role":"user","content":"hi"}]}"#
        )
        .expect("valid fixture JSON")
    );
    assert_eq!(sent.timeout, Duration::from_secs(6000));

    let zero = run_handle(&Call {
        timeout: Some(Duration::ZERO),
        ..Call::default()
    });
    assert_eq!(zero.seen.expect("a request").timeout, Duration::ZERO);

    let invalid_header = run_handle(&Call {
        api_key: SecretString::from("bad\nkey"),
        ..Call::default()
    });
    assert!(matches!(
        invalid_header.result,
        Err(ApiError::Processing { .. })
    ));
    assert!(
        invalid_header.seen.is_none(),
        "nothing is sent with an unbuildable header"
    );

    let missing_model = run_handle(&Call {
        model: String::new(),
        ..Call::default()
    });
    assert!(matches!(
        missing_model.result,
        Err(ApiError::Processing { .. })
    ));

    let not_a_list = run_handle(&Call {
        messages: Some(Json::Object(IndexMap::from([(
            "role".to_owned(),
            Json::Text("user".to_owned()),
        )]))),
        ..Call::default()
    });
    assert!(matches!(
        not_a_list.result,
        Err(ApiError::Processing { .. })
    ));

    let empty_messages_with_system = run_handle(&Call {
        messages: Some(Json::List(Vec::new())),
        system: Some(Json::Text("be brief".to_owned())),
        ..Call::default()
    });
    assert!(
        matches!(
            empty_messages_with_system.result,
            Err(ApiError::Processing { .. })
        ),
        "the Foundry handler validates the messages alone, so an empty list fails"
    );
    assert!(empty_messages_with_system.seen.is_none());
}

fn count_error_warnings() {
    let rate_limited = run_counter(&Call {
        reply: Some((429, "rate limited".to_owned())),
        ..Call::default()
    });
    assert_eq!(
        rate_limited.warnings,
        [Warning {
            subject: "Azure AI Anthropic CountTokens API error".to_owned(),
            status: Some(429),
        }]
    );
    assert!(
        rate_limited.result.is_some(),
        "an error record is not a missing result"
    );

    let not_json = run_counter(&Call {
        reply: Some((200, "not json".to_owned())),
        ..Call::default()
    });
    assert_eq!(
        not_json.warnings,
        [Warning {
            subject: "Error calling Azure AI Anthropic CountTokens API".to_owned(),
            status: None,
        }]
    );

    let success = run_counter(&Call::default());
    assert!(success.warnings.is_empty());
    assert!(success.result.is_some_and(|record| !record.error));

    let missing_key = run_counter(&Call {
        params: Params::Absent,
        ..Call::default()
    });
    assert_eq!(
        missing_key.warnings,
        [Warning {
            subject: "No Azure AI API key found for token counting".to_owned(),
            status: None,
        }]
    );
    assert!(missing_key.result.is_none());
}

#[test]
fn anthropic_count_credential_identity_isolation() {
    let first = run_counter(&Call {
        params: Params::Key(SecretString::from("first-params-key")),
        environment_key: Some(SecretString::from("first-environment-key")),
        ..Call::default()
    });
    let second = run_counter(&Call {
        params: Params::Absent,
        environment_key: Some(SecretString::from("second-environment-key")),
        ..Call::default()
    });
    let first_seen = first.seen.expect("a request");
    let second_seen = second.seen.expect("a request");
    assert_eq!(first_seen.headers["x-api-key"], "first-params-key");
    assert_eq!(first_seen.headers["api-key"], "first-params-key");
    assert_eq!(second_seen.headers["x-api-key"], "second-environment-key");
    assert_eq!(second_seen.headers["api-key"], "second-environment-key");

    let configured = run_handle(&Call {
        params: Params::Null,
        configured_key: Some(SecretString::from("first-configured-key")),
        ..Call::default()
    });
    let other_configured = run_handle(&Call {
        params: Params::Null,
        configured_key: Some(SecretString::from("second-configured-key")),
        ..Call::default()
    });
    let configured_seen = configured.seen.expect("a request");
    let other_seen = other_configured.seen.expect("a request");
    assert_eq!(configured_seen.headers["api-key"], "first-configured-key");
    assert_eq!(other_seen.headers["api-key"], "second-configured-key");
    assert_ne!(configured_seen.headers, other_seen.headers);
}

#[test]
fn anthropic_count_secret_safe_failure() {
    const SECRET: &str = "sk-foundry-fixture-secret-565";

    let invalid_header = run_handle(&Call {
        api_key: SecretString::from(format!("{SECRET}\n")),
        ..Call::default()
    });
    let failure = invalid_header
        .result
        .expect_err("the header cannot be built");
    assert!(format!("{failure:?}").contains("x-api-key"));
    assert!(!format!("{failure:?}").contains(SECRET));
    assert!(!failure.to_string().contains(SECRET));

    let counted = run_counter(&Call {
        params: Params::Key(SecretString::from(SECRET)),
        reply: Some((500, "upstream failure".to_owned())),
        ..Call::default()
    });
    let record = counted.result.expect("an error record");
    assert!(!format!("{record:?}").contains(SECRET));
    assert!(
        record
            .error_message
            .as_deref()
            .is_some_and(|message| !message.contains(SECRET))
    );

    let handler = AzureAnthropicCountTokensHandler {
        transport: Script {
            reply: None,
            seen: Arc::new(Mutex::new(Vec::new())),
        },
        request_timeout: Duration::from_secs(1),
        beta_version: DEFAULT_BETA_VERSION.to_owned(),
        configured_key: Some(SecretString::from(SECRET)),
    };
    let described = format!("{handler:?}");
    assert!(described.contains("[REDACTED]"));
    assert!(!described.contains(SECRET));
}

#[test]
fn replays_the_foundry_anthropic_count_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/foundry_anthropic_count");
    let paths: Vec<PathBuf> = std::fs::read_dir(&corpus)
        .expect("corpus folder")
        .map(|entry| entry.expect("corpus entry").path())
        .collect();
    assert!(!paths.is_empty());
    let failures: Vec<String> = paths
        .iter()
        .filter_map(|path| {
            let bytes = std::fs::read(path).expect("corpus file");
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("name");
            let decoded = decode_foundry_count_reply(&bytes);
            (decoded.is_ok() != name.starts_with("ok-")).then(|| name.to_owned())
        })
        .collect();
    assert!(failures.is_empty(), "{failures:?}");
    let object = decode_foundry_count_reply(br#"{"input_tokens":5}"#).expect("an object");
    assert!(matches!(object.document, Json::Object(_)));
    let null = decode_foundry_count_reply(b"null").expect("a null document");
    assert_eq!(null.document, Json::Null);
}

#[derive(Debug, PartialEq, Eq)]
struct Seen {
    url: String,
    headers: BTreeMap<String, String>,
    body: Json,
    timeout: Duration,
}

struct Script {
    reply: Option<(u16, String)>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Debug for Script {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Script").finish_non_exhaustive()
    }
}

impl CountTokensTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let OutboundRequest {
            url,
            headers,
            body,
            deadline,
            ..
        } = request;
        let payload = match body {
            RequestBody::Json(bytes) => {
                serde_json::from_slice(&bytes).expect("the request body is JSON")
            }
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Json::Null
            }
        };
        let pairs = headers
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen {
                url: url.to_string(),
                headers: pairs,
                body: payload,
                timeout: deadline,
            });
        ready(
            self.reply
                .clone()
                .map_or(Err(TransportError::Closed), |(status, body)| {
                    Ok(UpstreamResponse {
                        status,
                        headers: HeaderMap::new(),
                        body: body.into_bytes(),
                    })
                }),
        )
    }
}

// The litellm_params api_key entry as LiteLLM received it: missing, null, or a value
enum Params {
    Absent,
    Null,
    Key(SecretString),
    Typed(Option<SecretString>),
}

struct Call {
    api_key: SecretString,
    api_base: Option<String>,
    params: Params,
    configured_key: Option<SecretString>,
    token: Option<String>,
    environment_key: Option<SecretString>,
    environment_base: Option<String>,
    messages: Option<Json>,
    system: Option<Json>,
    tools: Option<Json>,
    model: String,
    request_model: String,
    timeout: Option<Duration>,
    request_timeout: Duration,
    environment: BTreeMap<String, String>,
    reply: Option<(u16, String)>,
}

impl Default for Call {
    fn default() -> Self {
        Self {
            api_key: SecretString::from(ARGUMENT_KEY),
            api_base: Some(API_BASE.to_owned()),
            params: Params::Key(SecretString::from(PARAMS_KEY)),
            configured_key: None,
            token: None,
            environment_key: None,
            environment_base: None,
            messages: Some(serde_json::from_str::<Json>(USER_MESSAGE).expect("valid fixture JSON")),
            system: None,
            tools: None,
            model: MODEL.to_owned(),
            request_model: MODEL.to_owned(),
            timeout: None,
            request_timeout: Duration::from_secs(6000),
            environment: BTreeMap::new(),
            reply: Some((200, r#"{"input_tokens":5}"#.to_owned())),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct CounterOutcome {
    selected: [bool; 3],
    result: Option<TokenCountResponse>,
    seen: Option<Seen>,
    warnings: Vec<Warning>,
}

#[derive(Debug)]
struct HandleOutcome {
    result: Result<Json, ApiError>,
    seen: Option<Seen>,
}

#[derive(Debug, PartialEq, Eq)]
struct Warning {
    subject: String,
    status: Option<u16>,
}

impl From<&Call> for AzureAnthropicCountTokensHandler<Script> {
    fn from(call: &Call) -> Self {
        Self {
            transport: Script {
                reply: call.reply.clone(),
                seen: Arc::new(Mutex::new(Vec::new())),
            },
            request_timeout: call.request_timeout,
            beta_version: beta_version(&call.environment),
            configured_key: call.configured_key.clone(),
        }
    }
}

fn run_counter(call: &Call) -> CounterOutcome {
    let credentials = credential_settings(call.token.as_deref());
    let token = TokenSource {
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &Offline,
    };
    let params_key = params_key(&call.params);
    let counter = AzureAnthropicTokenCounter {
        handler: AzureAnthropicCountTokensHandler::from(call),
        environment_api_key: call.environment_key.clone(),
        environment_api_base: call.environment_base.clone(),
    };
    let data = CountTokensCall {
        model_to_use: &call.model,
        messages: call.messages.as_ref(),
        contents: None,
        params_key,
        api_base: call.api_base.as_deref(),
        token: &token,
        request_model: &call.request_model,
        tools: call.tools.as_ref(),
        system: call.system.as_ref(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the test runtime");
    let (result, warnings) = captured(|| runtime.block_on(counter.count_tokens(&data)));
    let recorder = counter.handler.transport.seen.clone();
    let requests = recorder
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    CounterOutcome {
        selected: [
            counter.should_use_token_counting_api(Some("azure_ai")),
            counter.should_use_token_counting_api(Some("anthropic")),
            counter.should_use_token_counting_api(None),
        ],
        result,
        seen: requests.into_iter().next(),
        warnings,
    }
}

fn run_handle(call: &Call) -> HandleOutcome {
    let credentials = credential_settings(call.token.as_deref());
    let token = TokenSource {
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &Offline,
    };
    let params_key = params_key(&call.params);
    let messages = call.messages.clone().unwrap_or(Json::Null);
    let handler = AzureAnthropicCountTokensHandler::from(call);
    let request = CountTokensRequest {
        body: CountTokensBody {
            model: &call.model,
            messages: &messages,
            tools: call.tools.as_ref(),
            system: call.system.as_ref(),
            optional_params: &IndexMap::new(),
        },
        api_key: &call.api_key,
        api_base: call.api_base.as_deref().unwrap_or_default(),
        params_key,
        token: &token,
        timeout: call.timeout,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the test runtime");
    let outcome = runtime.block_on(handler.handle_count_tokens_request(&request));
    let recorder = handler.transport.seen.clone();
    let requests = recorder
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    HandleOutcome {
        result: outcome.map(|reply| reply.document),
        seen: requests.into_iter().next(),
    }
}

const fn params_key(params: &Params) -> ParamsKey<'_> {
    match params {
        Params::Absent => ParamsKey::DictAbsent,
        Params::Null => ParamsKey::DictNull,
        Params::Key(secret) => ParamsKey::Dict(secret),
        Params::Typed(None) => ParamsKey::Typed(None),
        Params::Typed(Some(secret)) => ParamsKey::Typed(Some(secret)),
    }
}

fn handle_headers(call: &Call) -> BTreeMap<String, String> {
    let outcome = run_handle(call);
    let seen = outcome
        .seen
        .expect("the fixture transport recorded a request");
    seen.headers
}

fn beta_version(environment: &BTreeMap<String, String>) -> String {
    let raw = BetaRaw::default();
    let settings = raw.convert(environment).expect("the beta settings");
    settings.beta_version
}

fn credential_settings(azure_ad_token: Option<&str>) -> DeploymentSettings {
    let token = azure_ad_token.map_or("null".to_owned(), |token| format!("{token:?}"));
    let source = format!(r#"{{"model_list":[{{"litellm_params":{{"azure_ad_token":{token}}}}}]}}"#);
    let raw: CredentialRaw = serde_json::from_str(&source).expect("the credential fixture");
    let settings = raw
        .convert(&BTreeMap::new())
        .expect("the credential settings");
    settings
        .deployments
        .into_iter()
        .next()
        .expect("one deployment")
}

// The count path never reaches Azure, so every token source refuses
struct Offline;

impl CredentialSources for Offline {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(SourceFailure {
            class: "ValueError".to_owned(),
            status: None,
            message: "the count path must not reach a token source".to_owned(),
        })
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(SourceFailure {
            class: "ValueError".to_owned(),
            status: None,
            message: "the count path must not reach a token source".to_owned(),
        })
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(SourceFailure {
            class: "ValueError".to_owned(),
            status: None,
            message: "the count path must not reach a token source".to_owned(),
        })
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(SourceFailure {
            class: "ValueError".to_owned(),
            status: None,
            message: "the count path must not reach a token source".to_owned(),
        })
    }
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);

impl Logs {
    fn entries(&self) -> Vec<Warning> {
        let guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut entries = Vec::with_capacity(guard.len());
        for message in guard.iter() {
            entries.push(warning(message));
        }
        entries
    }
}

fn warning(message: &str) -> Warning {
    let (subject, detail) = message.split_once(": ").unwrap_or((message, ""));
    let status = detail
        .strip_prefix("status=")
        .and_then(|rest| rest.split(',').next())
        .and_then(|number| number.parse::<u16>().ok());
    Warning {
        subject: subject.to_owned(),
        status,
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

fn captured<R>(body: impl FnOnce() -> R) -> (R, Vec<Warning>) {
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let outcome = with_default(subscriber, body);
    let entries = logs.entries();
    (outcome, entries)
}
