use std::time::Duration;

use betterlite_context::resource_config_selection::responses::ResponsesConfigTag;
use betterlite_error::envelope::WireField;
use betterlite_models::decisions::Json;
use betterlite_openai::response_resources::request::InputItemsOrder;
use betterlite_openai::response_resources::response::TypedReply;
use serde_json::Number;

use self::support::{
    Answer, DEFAULT_TIMEOUT, Fixture, caller_headers, input_items, json, map, retrieve,
};
use super::error::{LifecycleError, LifecycleFailure};
use super::input::{LifecyclePhases, LifecycleTimeout};
use crate::api::response_id_codecs::response::{build_response_id, decode_response_id};
use crate::api::responses_read_http::error::ReadError;
use crate::api::responses_read_http::service::ClientKind;

mod support;

const BODY: &str =
    r#"{"id":"resp_a","created_at":9,"status":"completed","model":"gpt-4o","output":[]}"#;
const ITEMS: &str =
    r#"{"object":"list","data":[{"id":"item_a","type":"message"}],"has_more":false}"#;
const DOD_404: &str = r#"{"error": {"message": "gone", "type": "invalid_request_error", "param": "response_id", "code": "not_found"}}"#;
const PLAIN_404: &str = r#"{"error": {"message": "gone"}}"#;
const CALL: &str = r#"{"response_id":"resp_a","custom_llm_provider":"openai","api_base":"http://mock.test/v1","api_key":"sk-provider","litellm_call_id":"call-a","litellm_metadata":{"model_info":{"id":"deployment-a"}}}"#;
const NO_CALL_ID: &str = r#"{"response_id":"resp_a","custom_llm_provider":"openai","api_base":"http://mock.test/v1","api_key":"sk-provider","litellm_metadata":{"model_info":{"id":"deployment-a"}}}"#;
const MISSING_PROVIDER: &str = r#"{"response_id":"resp_a"}"#;
const ANTHROPIC: &str = r#"{"response_id":"resp_a","custom_llm_provider":"anthropic"}"#;

// checks the rewritten id and the response's own fields, and hands the reply back for the header checks
fn retrieved(reply: TypedReply, model_id: &str, response_id: &str) -> TypedReply {
    assert_eq!(
        reply.response.id,
        build_response_id(Some("openai"), Some(model_id), response_id)
    );
    let decoded = decode_response_id(&reply.response.id);
    assert_eq!(
        decoded.custom_llm_provider.flatten().as_deref(),
        Some("openai")
    );
    assert_eq!(decoded.model_id.flatten().as_deref(), Some(model_id));
    assert_eq!(decoded.response_id.as_deref(), Some(response_id));
    assert_eq!(reply.response.created_at, 9_i64);
    assert_eq!(reply.response.status.as_deref(), Some("completed"));
    assert_eq!(reply.response.model.as_deref(), Some("gpt-4o"));
    reply
}

// checks that the exception's flat fields read as null rather than absent, and hands it back
fn null_fields(failure: LifecycleFailure) -> LifecycleFailure {
    assert_eq!(failure.kind, WireField::Null);
    assert_eq!(failure.param, WireField::Null);
    assert_eq!(failure.code, WireField::Null);
    failure
}

// the encoded id reaches the provider decoded, and the rewrite encodes the answered id exactly once
#[tokio::test]
async fn retrieve_routes_encoded_id() {
    let mut fixture = Fixture::new(vec![
        Answer::Retrieved {
            body: BODY,
            header: Some(("x-request-id", "req_a")),
        },
        Answer::Retrieved {
            body: BODY,
            header: Some(("x-request-id", "req_b")),
        },
        Answer::Retrieved {
            body: BODY,
            header: None,
        },
    ]);
    let encoded = build_response_id(Some("openai"), Some("deployment-a"), "resp/a");
    let mut call = retrieve(CALL);
    call.response_id = encoded;
    call.custom_llm_provider = Some(String::from("anthropic"));

    let reply = retrieved(
        fixture
            .retrieve(&call)
            .await
            .expect("the encoded retrieve succeeds"),
        "deployment-a",
        "resp/a",
    );
    assert_eq!(
        reply.headers.get("x-request-id").map(String::as_str),
        Some("req_a")
    );

    let awaited = retrieved(
        fixture
            .aretrieve(&call)
            .await
            .expect("the awaited retrieve succeeds"),
        "deployment-a",
        "resp/a",
    );
    assert_eq!(
        awaited.headers.get("x-request-id").map(String::as_str),
        Some("req_b")
    );

    let plain = retrieved(
        fixture
            .retrieve(&retrieve(CALL))
            .await
            .expect("the raw retrieve succeeds"),
        "deployment-a",
        "resp_a",
    );
    assert!(plain.headers.is_empty());

    let records = fixture.service.executor.records();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].response_id, "resp/a");
    assert_eq!(records[0].tag, ResponsesConfigTag::OpenAi);
    assert_eq!(records[0].api_base.as_deref(), Some("http://mock.test/v1"));
    assert_eq!(records[0].api_key.as_deref(), Some("sk-provider"));
    assert_eq!(records[0].mode, ClientKind::Sync);
    assert!(records[0].query.is_none());
    assert!(records[0].extra_body.is_none());
    assert_eq!(records[1].response_id, "resp/a");
    assert_eq!(records[1].mode, ClientKind::Asynchronous);
    assert_eq!(
        records[1].logged_litellm.get("litellm_call_id"),
        Some(&Json::Text(String::from("call-a")))
    );
    assert_eq!(records[2].response_id, "resp_a");
    assert_eq!(records[2].tag, ResponsesConfigTag::OpenAi);
    assert_eq!(records[2].mode, ClientKind::Sync);
}

// the five SDK pagination arguments travel as a query, and an absent limit and order keep the SDK defaults
#[tokio::test]
async fn input_items_sdk_pagination() {
    let mut fixture = Fixture::new(vec![
        Answer::Items(ITEMS),
        Answer::Items(ITEMS),
        Answer::Items(ITEMS),
        Answer::Items(ITEMS),
    ]);
    let defaults = input_items(CALL);
    let page = fixture
        .list(&defaults)
        .await
        .expect("the default listing succeeds");
    assert_eq!(page, json(ITEMS));

    let mut explicit = input_items(CALL);
    explicit.after = Some(String::new());
    explicit.before = Some(String::from("item_b"));
    explicit.include = Some(vec![String::from("message.input_image.image_url")]);
    explicit.limit = Some(2);
    explicit.order = Some(InputItemsOrder::Asc);
    let page = fixture
        .list(&explicit)
        .await
        .expect("the explicit listing succeeds");
    assert_eq!(page, json(ITEMS));

    let mut empty = input_items(CALL);
    empty.include = Some(Vec::new());
    empty.limit = None;
    empty.order = None;
    let page = fixture
        .list(&empty)
        .await
        .expect("the empty listing succeeds");
    assert_eq!(page, json(ITEMS));

    let awaited = fixture
        .alist(&defaults)
        .await
        .expect("the awaited listing succeeds");
    assert_eq!(awaited, json(ITEMS));

    let records = fixture.service.executor.records();
    assert_eq!(records.len(), 4);
    let Some(default_query) = &records[0].query else {
        panic!("the default listing sends a query");
    };
    assert_eq!(default_query.after, None);
    assert_eq!(default_query.before, None);
    assert_eq!(default_query.include, None);
    assert_eq!(default_query.limit, Some(20));
    assert_eq!(default_query.order, Some(InputItemsOrder::Desc));
    let Some(explicit_query) = &records[1].query else {
        panic!("the explicit listing sends a query");
    };
    assert_eq!(explicit_query.after.as_deref(), Some(""));
    assert_eq!(explicit_query.before.as_deref(), Some("item_b"));
    assert_eq!(
        explicit_query.include,
        Some(vec![String::from("message.input_image.image_url")])
    );
    assert_eq!(explicit_query.limit, Some(2));
    assert_eq!(explicit_query.order, Some(InputItemsOrder::Asc));
    let Some(empty_query) = &records[2].query else {
        panic!("the empty listing sends a query");
    };
    assert_eq!(empty_query.include, Some(Vec::new()));
    assert_eq!(empty_query.limit, None);
    assert_eq!(empty_query.order, None);
    assert_eq!(records[3].query, records[0].query);
    assert_eq!(records[3].mode, ClientKind::Asynchronous);
    assert_eq!(records[3].response_id, "resp_a");
    assert!(
        records
            .iter()
            .all(|record| record.tag == ResponsesConfigTag::OpenAi)
    );
    assert!(records.iter().all(|record| record.response_id == "resp_a"));
}

// extra_query never reaches the read seam, the timeout default follows the configured value, and only a JSON true flag is async
#[tokio::test]
async fn read_ignored_extras_and_context() {
    let mut fixture = Fixture::new(vec![
        Answer::Retrieved {
            body: BODY,
            header: None,
        },
        Answer::Retrieved {
            body: BODY,
            header: None,
        },
        Answer::Retrieved {
            body: BODY,
            header: None,
        },
        Answer::Retrieved {
            body: BODY,
            header: None,
        },
        Answer::Retrieved {
            body: BODY,
            header: None,
        },
        Answer::Retrieved {
            body: BODY,
            header: None,
        },
        Answer::Retrieved {
            body: BODY,
            header: None,
        },
        Answer::Retrieved {
            body: BODY,
            header: None,
        },
        Answer::Items(ITEMS),
        Answer::Items(ITEMS),
        Answer::Retrieved {
            body: BODY,
            header: None,
        },
    ]);
    let mut extras = retrieve(CALL);
    let headers = caller_headers(&[("X-Trace", "trace-a")]);
    assert_eq!(headers.get("X-Trace").map(String::as_str), Some("trace-a"));
    extras.extra_headers = Some(headers.clone());
    extras.extra_query = Some(map(r#"{"after":"ignored"}"#));
    extras.extra_body = Some(map(r#"{"ignored":true}"#));
    extras.timeout = Some(LifecycleTimeout::Seconds(Number::from(7)));
    extras.params.metadata = Some(map(r#"{"request_id":"r1"}"#));
    let _ = fixture
        .retrieve(&extras)
        .await
        .expect("the extras retrieve succeeds");

    let _ = fixture
        .retrieve(&retrieve(NO_CALL_ID))
        .await
        .expect("the default-timeout retrieve succeeds");
    let mut zero = retrieve(NO_CALL_ID);
    zero.timeout = Some(LifecycleTimeout::Seconds(Number::from(0)));
    let _ = fixture
        .retrieve(&zero)
        .await
        .expect("the zero-timeout retrieve succeeds");
    let mut phases = retrieve(NO_CALL_ID);
    phases.timeout = Some(LifecycleTimeout::Phases(LifecyclePhases {
        connect: None,
        read: Some(Number::from(9)),
        write: None,
        pool: None,
    }));
    let _ = fixture
        .retrieve(&phases)
        .await
        .expect("the phased retrieve succeeds");
    let mut no_read = retrieve(NO_CALL_ID);
    no_read.timeout = Some(LifecycleTimeout::Phases(LifecyclePhases {
        connect: Some(Number::from(3)),
        read: None,
        write: None,
        pool: None,
    }));
    let _ = fixture
        .retrieve(&no_read)
        .await
        .expect("the connect-only retrieve succeeds");

    let mut number_flag = retrieve(NO_CALL_ID);
    number_flag.aget_responses = Some(json("1"));
    let _ = fixture
        .retrieve(&number_flag)
        .await
        .expect("the number-flagged retrieve succeeds");
    let mut text_flag = retrieve(NO_CALL_ID);
    text_flag.aget_responses = Some(Json::Text(String::from("true")));
    let _ = fixture
        .retrieve(&text_flag)
        .await
        .expect("the text-flagged retrieve succeeds");
    let mut bool_flag = retrieve(NO_CALL_ID);
    bool_flag.aget_responses = Some(Json::Bool(true));
    let _ = fixture
        .retrieve(&bool_flag)
        .await
        .expect("the bool-flagged retrieve succeeds");
    let mut items_number = input_items(NO_CALL_ID);
    items_number.alist_input_items = Some(json("1"));
    let _ = fixture
        .list(&items_number)
        .await
        .expect("the number-flagged listing succeeds");
    let mut items_bool = input_items(NO_CALL_ID);
    items_bool.alist_input_items = Some(Json::Bool(true));
    let _ = fixture
        .alist(&items_bool)
        .await
        .expect("the bool-flagged listing succeeds");

    let mut zero_read = retrieve(NO_CALL_ID);
    zero_read.timeout = Some(LifecycleTimeout::Phases(LifecyclePhases {
        connect: None,
        read: Some(Number::from(0)),
        write: None,
        pool: None,
    }));
    let _ = fixture
        .retrieve(&zero_read)
        .await
        .expect("the zero-read retrieve succeeds");

    let records = fixture.service.executor.records();
    assert_eq!(records.len(), 11);
    assert_eq!(records[0].extra_headers, Some(headers));
    assert_eq!(records[0].extra_body, Some(map(r#"{"ignored":true}"#)));
    assert!(records[0].query.is_none());
    assert_eq!(records[0].timeout, Some(Duration::from_secs(7)));
    assert_eq!(records[0].mode, ClientKind::Sync);
    assert_eq!(
        records[0].logged_optional,
        map(r#"{"response_id":"resp_a"}"#)
    );
    assert_eq!(
        records[0].logged_litellm.get("litellm_call_id"),
        Some(&Json::Text(String::from("call-a")))
    );
    assert_eq!(
        records[0].logged_litellm.get("metadata"),
        Some(&Json::Object(map(r#"{"request_id":"r1"}"#)))
    );
    assert_eq!(
        records[0].logged_litellm.get("litellm_metadata"),
        Some(&Json::Object(map(
            r#"{"model_info":{"id":"deployment-a"}}"#
        )))
    );
    assert_eq!(records[1].timeout, Some(DEFAULT_TIMEOUT));
    assert_eq!(
        records[1].logged_litellm.get("litellm_call_id"),
        Some(&Json::Null)
    );
    assert_eq!(records[2].timeout, Some(DEFAULT_TIMEOUT));
    assert_eq!(records[3].timeout, Some(Duration::from_secs(9)));
    assert_eq!(records[4].timeout, Some(DEFAULT_TIMEOUT));
    assert_eq!(records[5].mode, ClientKind::Sync);
    assert_eq!(records[6].mode, ClientKind::Sync);
    assert_eq!(records[7].mode, ClientKind::Asynchronous);
    assert_eq!(
        records[7].logged_optional,
        map(r#"{"response_id":"resp_a"}"#)
    );
    assert_eq!(records[8].mode, ClientKind::Sync);
    assert_eq!(records[9].mode, ClientKind::Asynchronous);
    assert_eq!(records[10].timeout, Some(DEFAULT_TIMEOUT));
    assert_eq!(records[10].mode, ClientKind::Sync);
    assert!(records[10].query.is_none());
    assert!(records.iter().take(8).all(|record| record.query.is_none()));
    assert_eq!(fixture.state().model, "responses");
    assert_eq!(
        fixture.state().custom_llm_provider.as_deref(),
        Some("openai")
    );

    let mut injected = Fixture::with_timeout(
        vec![Answer::Retrieved {
            body: BODY,
            header: None,
        }],
        Duration::from_secs(17),
    );
    let _ = injected
        .retrieve(&retrieve(NO_CALL_ID))
        .await
        .expect("the injected-default retrieve succeeds");
    assert_eq!(
        injected.service.executor.records()[0].timeout,
        Some(Duration::from_secs(17))
    );
}

// every locally raised and provider error keeps its class, status and flat fields, and the source cause survives
#[tokio::test]
async fn read_provider_errors() {
    let mut fixture = Fixture::new(vec![
        Answer::Provider {
            status: 404,
            message: DOD_404,
        },
        Answer::Provider {
            status: 404,
            message: PLAIN_404,
        },
        Answer::Provider {
            status: 500,
            message: "not json",
        },
        Answer::Provider {
            status: 401,
            message: "unauthorized",
        },
    ]);
    let missing = null_fields(
        fixture
            .retrieve(&retrieve(MISSING_PROVIDER))
            .await
            .expect_err("the missing provider fails before HTTP"),
    );
    assert_eq!(missing.class, "APIConnectionError");
    assert_eq!(missing.status, 500);
    assert_eq!(
        missing.message,
        "Exception - custom_llm_provider is required but passed as None"
    );
    assert_eq!(
        missing.error.to_string(),
        "custom_llm_provider is required but passed as None"
    );

    let awaited = null_fields(
        fixture
            .aretrieve(&retrieve(MISSING_PROVIDER))
            .await
            .expect_err("the awaited missing provider fails the same way"),
    );
    assert_eq!(awaited.class, missing.class);
    assert_eq!(awaited.status, missing.status);
    assert_eq!(awaited.message, missing.message);

    let unsupported = null_fields(
        fixture
            .retrieve(&retrieve(ANTHROPIC))
            .await
            .expect_err("the unsupported provider fails before HTTP"),
    );
    assert_eq!(unsupported.class, "APIConnectionError");
    assert_eq!(unsupported.status, 500);
    assert_eq!(
        unsupported.message,
        "AnthropicException - GET responses is not supported for anthropic"
    );
    let unsupported_items = fixture
        .list(&input_items(ANTHROPIC))
        .await
        .expect_err("the unsupported listing fails before HTTP");
    assert_eq!(
        unsupported_items.message,
        "AnthropicException - list_input_items is not supported for anthropic"
    );
    let unsupported_awaited = fixture
        .alist(&input_items(ANTHROPIC))
        .await
        .expect_err("the awaited unsupported listing fails before HTTP");
    assert_eq!(unsupported_awaited.class, unsupported_items.class);
    assert_eq!(unsupported_awaited.message, unsupported_items.message);
    assert!(fixture.service.executor.records().is_empty());

    let bad = null_fields(
        fixture
            .retrieve(&retrieve(CALL))
            .await
            .expect_err("the 404 fails"),
    );
    assert_eq!(bad.class, "BadRequestError");
    assert_eq!(bad.status, 404);
    assert_eq!(bad.message, format!("OpenAIException - {DOD_404}"));
    let LifecycleError::Read(ReadError::Provider(source)) = &bad.error else {
        panic!("the provider cause is kept, got {:?}", bad.error);
    };
    assert_eq!(source.status_code, 404);
    assert_eq!(source.message, DOD_404);

    let plain = null_fields(
        fixture
            .retrieve(&retrieve(CALL))
            .await
            .expect_err("the plain 404 fails"),
    );
    assert_eq!(plain.class, "NotFoundError");
    assert_eq!(plain.status, 404);
    assert_eq!(
        plain.message,
        format!("NotFoundError: OpenAIException - {PLAIN_404}")
    );

    let server = fixture
        .retrieve(&retrieve(CALL))
        .await
        .expect_err("the 500 fails");
    assert_eq!(server.class, "InternalServerError");
    assert_eq!(server.status, 500);
    assert_eq!(
        server.kind,
        WireField::Text(String::from("internal_server_error"))
    );
    assert_eq!(server.param, WireField::Null);
    assert_eq!(server.code, WireField::Null);
    assert_eq!(
        server.message,
        "InternalServerError: OpenAIException - not json"
    );

    let auth = null_fields(
        fixture
            .retrieve(&retrieve(CALL))
            .await
            .expect_err("the 401 fails"),
    );
    assert_eq!(auth.class, "AuthenticationError");
    assert_eq!(auth.status, 401);
    assert_eq!(
        auth.message,
        "AuthenticationError: OpenAIException - unauthorized"
    );
    assert_eq!(fixture.service.executor.records().len(), 4);
}
