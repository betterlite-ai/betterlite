#![cfg(test)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_anthropic::count_tokens::wire::CountResponse;
use betterlite_config::sections::anthropic_count_tokens::DEFAULT_BETA_VERSION;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use crate::anthropic_count::endpoint::get_count_tokens_endpoint;
use crate::anthropic_count::error::ApiError;
use crate::anthropic_count::handler::FoundryCountHandler;
use crate::anthropic_count::reply::decode_foundry_count_reply;

mod support;

use support::{
    ARGUMENT_KEY, Call, MODEL, PARAMS_KEY, Params, Script, TOKENIZER_TYPE, Warning, beta_version,
    handle_headers, run_counter, run_handle,
};

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

    let plain = handle_headers(&Call::standard());
    assert_eq!(plain["content-type"], "application/json");
    assert_eq!(plain["anthropic-version"], "2023-06-01");
    assert_eq!(plain["anthropic-beta"], beta.as_str());
    assert_eq!(plain["x-api-key"], ARGUMENT_KEY);
    assert_eq!(plain["api-key"], PARAMS_KEY);

    let absent = handle_headers(&Call {
        params: Params::Absent,
        ..Call::standard()
    });
    assert_eq!(absent["x-api-key"], ARGUMENT_KEY);
    assert_eq!(absent["api-key"], ARGUMENT_KEY);

    let other = Call {
        params: Params::Key(SecretString::from("other-params-key")),
        ..Call::standard()
    };
    let different = handle_headers(&other);
    assert_eq!(different["x-api-key"], ARGUMENT_KEY);
    assert_eq!(different["api-key"], "other-params-key");

    let configured = Call {
        params: Params::Null,
        configured_key: Some(SecretString::from("configured-key")),
        ..Call::standard()
    };
    let nulled = handle_headers(&configured);
    assert_eq!(nulled["x-api-key"], ARGUMENT_KEY);
    assert_eq!(nulled["api-key"], "configured-key");

    let bearer = Call {
        params: Params::Null,
        token: Some("azure-token".to_owned()),
        ..Call::standard()
    };
    let entranced = handle_headers(&bearer);
    assert_eq!(entranced["x-api-key"], ARGUMENT_KEY);
    assert_eq!(entranced["authorization"], "Bearer azure-token");
    assert!(!entranced.contains_key("api-key"));

    let typed = Call {
        params: Params::Typed(Some(SecretString::from("typed-key"))),
        ..Call::standard()
    };
    let typed_headers = handle_headers(&typed);
    assert_eq!(typed_headers["x-api-key"], ARGUMENT_KEY);
    assert_eq!(typed_headers["api-key"], "typed-key");

    let typed_empty = Call {
        params: Params::Typed(Some(SecretString::from(""))),
        ..Call::standard()
    };
    let typed_empty_headers = handle_headers(&typed_empty);
    assert_eq!(typed_empty_headers["x-api-key"], ARGUMENT_KEY);
    assert_eq!(typed_empty_headers["api-key"], ARGUMENT_KEY);

    let empty_key_configured = Call {
        params: Params::Key(SecretString::from("")),
        configured_key: Some(SecretString::from("configured-key")),
        ..Call::standard()
    };
    let empty_key_headers = handle_headers(&empty_key_configured);
    assert_eq!(empty_key_headers["x-api-key"], ARGUMENT_KEY);
    assert_eq!(empty_key_headers["api-key"], "configured-key");
}

fn native_count_result() {
    let success = run_counter(&Call::standard());
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
        ..Call::standard()
    });
    assert_eq!(missing.result.expect("a token count").total_tokens, 0);

    let nulled = run_counter(&Call {
        reply: Some((200, "null".to_owned())),
        ..Call::standard()
    });
    assert!(nulled.result.is_none());

    let empty = run_counter(&Call {
        messages: Some(Json::List(Vec::new())),
        ..Call::standard()
    });
    assert!(empty.result.is_none());
    assert!(
        empty.seen.is_none(),
        "falsey messages never reach the transport"
    );

    let absent = run_counter(&Call {
        messages: None,
        ..Call::standard()
    });
    assert!(absent.result.is_none());
    assert!(
        absent.seen.is_none(),
        "contents never stands in for messages"
    );

    for falsey_messages in ["null", "false", "0", "\"\"", "[]", "{}"] {
        let skipped = run_counter(&Call {
            messages: Some(
                serde_json::from_str::<Json>(falsey_messages).expect("valid fixture JSON"),
            ),
            ..Call::standard()
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
        ..Call::standard()
    });
    let environment_seen = from_environment.seen.expect("a request");
    assert_eq!(
        environment_seen.url,
        "https://env.services.ai.azure.com/anthropic/v1/messages/count_tokens"
    );
    assert_eq!(environment_seen.headers["x-api-key"], "environment-key");

    let deployment = run_counter(&Call::standard());
    let deployment_seen = deployment.seen.expect("a request");
    assert_eq!(
        deployment_seen.url,
        "https://r.services.ai.azure.com/anthropic/v1/messages/count_tokens"
    );
    assert_eq!(deployment_seen.headers["x-api-key"], PARAMS_KEY);

    let no_key = run_counter(&Call {
        params: Params::Absent,
        ..Call::standard()
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
        ..Call::standard()
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
        ..Call::standard()
    });
    let empty_seen = empty.seen.expect("a request");
    assert_eq!(empty_seen.headers["x-api-key"], "environment-key");

    let empty_environment_key = run_counter(&Call {
        params: Params::Absent,
        environment_key: Some(SecretString::from("")),
        ..Call::standard()
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
        ..Call::standard()
    });
    let empty_deployment_seen = empty_deployment_base.seen.expect("a request");
    assert_eq!(
        empty_deployment_seen.url,
        "https://env.services.ai.azure.com/anthropic/v1/messages/count_tokens"
    );

    let empty_environment_base = run_counter(&Call {
        api_base: None,
        environment_base: Some(String::new()),
        ..Call::standard()
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
        ..Call::standard()
    });
    assert!(
        credentials_only.result.is_none(),
        "Entra credentials never replace the key"
    );
    assert!(credentials_only.seen.is_none());

    let configured_only = run_counter(&Call {
        params: Params::Absent,
        configured_key: Some(SecretString::from("configured-key")),
        ..Call::standard()
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
        ..Call::standard()
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
        ..Call::standard()
    });
    let bad_record = bad_request.result.expect("an error record");
    assert_eq!(bad_record.status_code, Some(400));
    assert_eq!(
        bad_record.error_message.as_deref(),
        Some("{\"error\":\"bad\"}")
    );

    let created = run_counter(&Call {
        reply: Some((201, "{\"input_tokens\":5}".to_owned())),
        ..Call::standard()
    });
    let created_record = created.result.expect("an error record");
    assert_eq!(created_record.status_code, Some(201));
    assert_eq!(
        created_record.error_message.as_deref(),
        Some("{\"input_tokens\":5}")
    );

    let not_json = run_counter(&Call {
        reply: Some((200, "not json".to_owned())),
        ..Call::standard()
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
        ..Call::standard()
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
        ..Call::standard()
    });
    assert_eq!(
        null_tokens.result.expect("an error record").status_code,
        Some(500)
    );

    let failed_transport = run_counter(&Call {
        reply: None,
        ..Call::standard()
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
    let sent = run_handle(&Call::standard()).seen.expect("a request");
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
        ..Call::standard()
    });
    assert_eq!(zero.seen.expect("a request").timeout, Duration::ZERO);

    let invalid_header = run_handle(&Call {
        api_key: SecretString::from("bad\nkey"),
        ..Call::standard()
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
        ..Call::standard()
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
        ..Call::standard()
    });
    assert!(matches!(
        not_a_list.result,
        Err(ApiError::Processing { .. })
    ));

    let empty_messages_with_system = run_handle(&Call {
        messages: Some(Json::List(Vec::new())),
        system: Some(Json::Text("be brief".to_owned())),
        ..Call::standard()
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
        ..Call::standard()
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
        ..Call::standard()
    });
    assert_eq!(
        not_json.warnings,
        [Warning {
            subject: "Error calling Azure AI Anthropic CountTokens API".to_owned(),
            status: None,
        }]
    );

    let success = run_counter(&Call::standard());
    assert!(success.warnings.is_empty());
    assert!(success.result.is_some_and(|record| !record.error));

    let missing_key = run_counter(&Call {
        params: Params::Absent,
        ..Call::standard()
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
        ..Call::standard()
    });
    let second = run_counter(&Call {
        params: Params::Absent,
        environment_key: Some(SecretString::from("second-environment-key")),
        ..Call::standard()
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
        ..Call::standard()
    });
    let other_configured = run_handle(&Call {
        params: Params::Null,
        configured_key: Some(SecretString::from("second-configured-key")),
        ..Call::standard()
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
        ..Call::standard()
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
        ..Call::standard()
    });
    let record = counted.result.expect("an error record");
    assert!(!format!("{record:?}").contains(SECRET));
    assert!(
        record
            .error_message
            .as_deref()
            .is_some_and(|message| !message.contains(SECRET))
    );

    let handler = FoundryCountHandler {
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
