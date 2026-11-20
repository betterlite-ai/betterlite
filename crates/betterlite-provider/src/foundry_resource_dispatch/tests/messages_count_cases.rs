use std::time::Duration;

use betterlite_anthropic::count_tokens::wire::CountTokensBody;
use betterlite_foundry::anthropic_count::error::ApiError;
use betterlite_foundry::anthropic_count::handler::{CountTokensRequest, FoundryCountHandler};
use betterlite_foundry::anthropic_messages::environment::{ParamsKey, TokenSource};
use betterlite_foundry::anthropic_messages::url_error::UrlError;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::fixture::{CountTransport, NoSources, credentials, parse};
use crate::foundry_resource_dispatch::messages_count::{self, Call, Dispatch, Outcome};
use crate::foundry_resource_dispatch::{
    FoundryResourceError, FoundryResourceOperation, FoundryResourceResult, dispatch,
};

fn handler(status: u16, body: Vec<u8>) -> FoundryCountHandler<CountTransport> {
    FoundryCountHandler {
        transport: CountTransport { status, body },
        request_timeout: Duration::from_secs(5),
        beta_version: String::from("2024-06-01"),
        configured_key: None,
    }
}

async fn run_with<'call>(
    handler: &'call FoundryCountHandler<CountTransport>,
    call: Call<'call>,
) -> Result<FoundryResourceResult, FoundryResourceError> {
    dispatch(FoundryResourceOperation::MessagesCount(
        Dispatch { handler, call }.run(),
    ))
    .await
}

fn outcome(result: Result<FoundryResourceResult, FoundryResourceError>) -> Outcome {
    match result {
        Ok(FoundryResourceResult::MessagesCount(outcome)) => outcome,
        Ok(_) | Err(_) => panic!("the messages call returns its own outcome"),
    }
}

fn leaf_error(
    result: Result<FoundryResourceResult, FoundryResourceError>,
) -> messages_count::Error {
    match result {
        Err(FoundryResourceError::MessagesCount(error)) => error,
        Ok(_) | Err(_) => panic!("the messages call fails in its leaf"),
    }
}

async fn policy_row() {
    let handler = handler(200, Vec::new());
    let Outcome::Policy {
        provider,
        strips_billing_metadata,
    } = outcome(run_with(&handler, Call::Policy).await)
    else {
        panic!("the policy call returns the native messages policy");
    };
    assert_eq!(provider, "azure_ai");
    assert!(strips_billing_metadata);
}

async fn url_rows() {
    let handler = handler(200, Vec::new());
    let Outcome::MessagesUrl(messages) = outcome(
        run_with(
            &handler,
            Call::MessagesUrl {
                api_base: Some("https://res.test/anthropic"),
                environment_api_base: None,
            },
        )
        .await,
    ) else {
        panic!("the messages url call returns its endpoint");
    };
    assert_eq!(messages, "https://res.test/anthropic/v1/messages");
    let Outcome::MessagesUrl(from_environment) = outcome(
        run_with(
            &handler,
            Call::MessagesUrl {
                api_base: Some(""),
                environment_api_base: Some("https://env.test"),
            },
        )
        .await,
    ) else {
        panic!("the messages url call returns its endpoint");
    };
    assert_eq!(from_environment, "https://env.test/anthropic/v1/messages");
    let error = leaf_error(
        run_with(
            &handler,
            Call::MessagesUrl {
                api_base: None,
                environment_api_base: None,
            },
        )
        .await,
    );
    assert!(matches!(
        error,
        messages_count::Error::Url(UrlError::MissingApiBase)
    ));

    let Outcome::CountUrl(count) = outcome(
        run_with(
            &handler,
            Call::CountUrl {
                api_base: "https://res.test/anthropic/",
            },
        )
        .await,
    ) else {
        panic!("the count url call returns its endpoint");
    };
    assert_eq!(count, "https://res.test/anthropic/v1/messages/count_tokens");
    let Outcome::CountUrl(plain) = outcome(
        run_with(
            &handler,
            Call::CountUrl {
                api_base: "https://res.test",
            },
        )
        .await,
    ) else {
        panic!("the count url call returns its endpoint");
    };
    assert_eq!(plain, "https://res.test/anthropic/v1/messages/count_tokens");
}

async fn transform_rows() {
    let handler = handler(200, Vec::new());
    let mut request: Object = parse(
        r#"{"model":"claude-sonnet-4-5","system":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral","scope":"global"}}],"messages":[{"role":"user","content":[{"type":"text","text":"yo","cache_control":{"type":"ephemeral","scope":"org"}}]}]}"#,
    );
    let Outcome::Transformed(transformed) = outcome(
        run_with(
            &handler,
            Call::Transform {
                request: &mut request,
            },
        )
        .await,
    ) else {
        panic!("the transform call returns the transformed request");
    };
    let expected: Object = parse(
        r#"{"model":"claude-sonnet-4-5","system":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}],"messages":[{"role":"user","content":[{"type":"text","text":"yo","cache_control":{"type":"ephemeral"}}]}]}"#,
    );
    assert_eq!(transformed, expected);
    assert_eq!(request, expected);
}

async fn count_rows() {
    let handler = handler(200, br#"{"input_tokens":7}"#.to_vec());
    let messages: Json = parse(r#"[{"role":"user","content":"hi"}]"#);
    let optional = IndexMap::new();
    let key = SecretString::from("sk-azure");
    let settings = credentials();
    let sources = NoSources;
    let token = TokenSource {
        credentials: &settings,
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    };
    let request = CountTokensRequest {
        body: CountTokensBody {
            model: "claude-sonnet-4-5",
            messages: &messages,
            tools: None,
            system: None,
            optional_params: &optional,
        },
        api_key: &key,
        api_base: "https://res.test",
        params_key: ParamsKey::DictAbsent,
        token: &token,
        timeout: None,
    };
    let Outcome::Count(reply) = outcome(run_with(&handler, Call::Count(request)).await) else {
        panic!("the count call returns the decoded reply");
    };
    let expected: Json = parse(r#"{"input_tokens":7}"#);
    assert_eq!(reply.document, expected);

    let failing = handler(500, b"boom".to_vec());
    let request = CountTokensRequest {
        body: CountTokensBody {
            model: "claude-sonnet-4-5",
            messages: &messages,
            tools: None,
            system: None,
            optional_params: &optional,
        },
        api_key: &key,
        api_base: "https://res.test",
        params_key: ParamsKey::DictAbsent,
        token: &token,
        timeout: None,
    };
    let error = leaf_error(run_with(&failing, Call::Count(request)).await);
    assert!(matches!(
        error,
        messages_count::Error::Count(ApiError::Upstream {
            status_code: 500,
            ..
        })
    ));
}

pub(super) async fn check() {
    policy_row().await;
    url_rows().await;
    transform_rows().await;
    count_rows().await;
}
