use http::HeaderMap;
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use rstest::rstest;
use serde_json::{Value, json};

use super::support::{CountingTransport, TOKEN_BODY, auth, environment, options, secret};
use crate::watsonx_auth::error::WatsonxError;
use crate::watsonx_chat::error::WatsonxChatError;
use crate::watsonx_chat::plan::{WatsonxChatPlan, WatsonxChatRequest};

const HOST: &str = "https://host.example";

fn bearer_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer caller"));
    headers
}

fn request(model: &str, headers: HeaderMap, given: Value) -> WatsonxChatRequest<'_> {
    WatsonxChatRequest {
        model,
        api_base: Some(HOST),
        api_key: None,
        headers,
        options: options(given),
    }
}

fn body(plan: &WatsonxChatPlan) -> Value {
    serde_json::to_value(&plan.options).unwrap()
}

#[tokio::test]
async fn plan_puts_model_and_project_id_in_the_body() {
    let client = auth(environment(None, None), CountingTransport::closed());
    let plan = WatsonxChatPlan::prepare(
        &client,
        request(
            "granite-13b",
            bearer_headers(),
            json!({"project_id": "project-1", "temperature": 0.5, "region_name": "us-south"}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(plan.model, "granite-13b");
    assert_eq!(
        plan.url,
        format!("{HOST}/ml/v1/text/chat?version=2024-03-13")
    );
    assert_eq!(
        body(&plan),
        json!({"temperature": 0.5, "model_id": "granite-13b", "project_id": "project-1"})
    );
    assert_eq!(client.transport.calls(), 0);
}

#[tokio::test]
async fn plan_falls_back_to_the_space_id() {
    let client = auth(environment(None, None), CountingTransport::closed());
    let plan = WatsonxChatPlan::prepare(
        &client,
        request(
            "granite-13b",
            bearer_headers(),
            json!({"space_id": "space-1"}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        body(&plan),
        json!({"model_id": "granite-13b", "space_id": "space-1"})
    );
}

#[tokio::test]
async fn plan_prefers_the_project_over_the_space() {
    let client = auth(environment(None, None), CountingTransport::closed());
    let plan = WatsonxChatPlan::prepare(
        &client,
        request(
            "granite-13b",
            bearer_headers(),
            json!({"space_id": "space-1", "project_id": "project-1"}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        body(&plan),
        json!({"model_id": "granite-13b", "project_id": "project-1"})
    );
}

#[tokio::test]
async fn plan_reads_the_project_from_the_environment() {
    let client = auth(
        environment(None, Some("env-project")),
        CountingTransport::closed(),
    );
    let plan =
        WatsonxChatPlan::prepare(&client, request("granite-13b", bearer_headers(), json!({})))
            .await
            .unwrap();
    assert_eq!(
        body(&plan),
        json!({"model_id": "granite-13b", "project_id": "env-project"})
    );
}

#[tokio::test]
async fn plan_overwrites_ids_the_caller_left_in_the_options() {
    let client = auth(environment(None, None), CountingTransport::closed());
    let plan = WatsonxChatPlan::prepare(
        &client,
        request(
            "granite-13b",
            bearer_headers(),
            json!({"model_id": "other", "project_id": "project-1", "space_id": "space-1"}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        body(&plan),
        json!({"model_id": "granite-13b", "project_id": "project-1", "space_id": "space-1"})
    );
}

#[rstest]
#[case::plain("deployment/abc", false, "/ml/v1/deployments/abc/text/chat")]
#[case::stream("deployment/abc", true, "/ml/v1/deployments/abc/text/chat_stream")]
#[tokio::test]
async fn plan_for_a_deployment_has_an_empty_model_and_no_ids(
    #[case] model: &str,
    #[case] stream: bool,
    #[case] path: &str,
) {
    let client = auth(environment(None, None), CountingTransport::closed());
    let plan = WatsonxChatPlan::prepare(
        &client,
        request(
            model,
            bearer_headers(),
            json!({"project_id": "project-1", "stream": stream, "temperature": 0.1}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(plan.model, "");
    assert_eq!(plan.url, format!("{HOST}{path}?version=2024-03-13"));
    assert_eq!(body(&plan), json!({"stream": stream, "temperature": 0.1}));
}

#[tokio::test]
async fn plan_without_project_or_space_fails_with_401_before_any_send() {
    let client = auth(
        environment(None, None),
        CountingTransport::answering(TOKEN_BODY),
    );
    let error = WatsonxChatPlan::prepare(
        &client,
        WatsonxChatRequest {
            model: "granite-13b",
            api_base: Some(HOST),
            api_key: Some(&secret("key")),
            headers: HeaderMap::new(),
            options: options(json!({})),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        WatsonxChatError::Auth(WatsonxError::Api {
            status_code: 401,
            ..
        })
    ));
    assert_eq!(client.transport.calls(), 0);
}

#[tokio::test]
async fn plan_for_a_deployment_needs_no_project() {
    let client = auth(environment(None, None), CountingTransport::closed());
    let plan = WatsonxChatPlan::prepare(
        &client,
        request("deployment/abc", bearer_headers(), json!({})),
    )
    .await
    .unwrap();
    assert_eq!(plan.model, "");
    assert!(plan.options.is_empty());
}

#[tokio::test]
async fn plan_headers_keep_the_caller_authorization_and_add_the_defaults() {
    let client = auth(environment(None, None), CountingTransport::closed());
    let plan = WatsonxChatPlan::prepare(
        &client,
        request("granite-13b", bearer_headers(), json!({"project_id": "p"})),
    )
    .await
    .unwrap();
    assert_eq!(plan.headers.get(AUTHORIZATION).unwrap(), "Bearer caller");
    assert_eq!(plan.headers.get(CONTENT_TYPE).unwrap(), "application/json");
    assert_eq!(plan.headers.get(ACCEPT).unwrap(), "application/json");
}

#[tokio::test]
async fn plan_generates_the_iam_token_when_no_authorization_is_given() {
    let client = auth(
        environment(None, None),
        CountingTransport::answering(TOKEN_BODY),
    );
    let key = secret("api-key");
    let plan = WatsonxChatPlan::prepare(
        &client,
        WatsonxChatRequest {
            model: "granite-13b",
            api_base: Some(HOST),
            api_key: Some(&key),
            headers: HeaderMap::new(),
            options: options(json!({"project_id": "p"})),
        },
    )
    .await
    .unwrap();
    assert_eq!(plan.headers.get(AUTHORIZATION).unwrap(), "Bearer iam-token");
    assert_eq!(client.transport.calls(), 1);
}

#[tokio::test]
async fn plan_resolves_the_token_before_the_url() {
    let client = auth(environment(None, None), CountingTransport::closed());
    let key = secret("api-key");
    let error = WatsonxChatPlan::prepare(
        &client,
        WatsonxChatRequest {
            model: "granite-13b",
            api_base: None,
            api_key: Some(&key),
            headers: HeaderMap::new(),
            options: options(json!({"project_id": "p"})),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        WatsonxChatError::Auth(WatsonxError::IamTransport(_))
    ));
}

#[tokio::test]
async fn plan_without_any_base_fails_with_401() {
    let client = auth(environment(None, None), CountingTransport::closed());
    let error = WatsonxChatPlan::prepare(
        &client,
        WatsonxChatRequest {
            model: "granite-13b",
            api_base: None,
            api_key: None,
            headers: bearer_headers(),
            options: options(json!({"project_id": "p"})),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        WatsonxChatError::Auth(WatsonxError::Api {
            status_code: 401,
            ..
        })
    ));
}

#[rstest]
#[case::absent(json!({}), "/ml/v1/text/chat")]
#[case::null(json!({"stream": null}), "/ml/v1/text/chat")]
#[case::off(json!({"stream": false}), "/ml/v1/text/chat")]
#[case::on(json!({"stream": true}), "/ml/v1/text/chat_stream")]
#[tokio::test]
async fn plan_reads_stream_from_the_options(#[case] given: Value, #[case] path: &str) {
    let client = auth(environment(None, None), CountingTransport::closed());
    let mut given_options = options(given.clone());
    given_options.insert("project_id".to_owned(), json!("p"));
    let plan = WatsonxChatPlan::prepare(
        &client,
        WatsonxChatRequest {
            model: "granite-13b",
            api_base: Some(HOST),
            api_key: None,
            headers: bearer_headers(),
            options: given_options,
        },
    )
    .await
    .unwrap();
    assert_eq!(plan.url, format!("{HOST}{path}?version=2024-03-13"));
    assert_eq!(plan.options.get("stream"), given.get("stream"));
}

#[rstest]
#[case::text(json!("true"))]
#[case::number(json!(1))]
#[case::list(json!([true]))]
#[case::object(json!({}))]
#[tokio::test]
async fn plan_rejects_a_stream_that_is_not_a_boolean(#[case] stream: Value) {
    let client = auth(environment(None, None), CountingTransport::closed());
    let error = WatsonxChatPlan::prepare(
        &client,
        request(
            "granite-13b",
            bearer_headers(),
            json!({"project_id": "p", "stream": stream}),
        ),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        WatsonxChatError::InvalidField { field: "stream" }
    ));
}

#[tokio::test]
async fn plan_pops_the_api_version_into_the_url() {
    let client = auth(environment(None, None), CountingTransport::closed());
    let plan = WatsonxChatPlan::prepare(
        &client,
        request(
            "granite-13b",
            bearer_headers(),
            json!({"project_id": "p", "api_version": "2025-02-02"}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        plan.url,
        format!("{HOST}/ml/v1/text/chat?version=2025-02-02")
    );
    assert!(!plan.options.contains_key("api_version"));
}

#[tokio::test]
async fn plan_consumes_the_token_and_zen_key_options() {
    let client = auth(environment(None, None), CountingTransport::closed());
    let token_plan = WatsonxChatPlan::prepare(
        &client,
        request(
            "granite-13b",
            HeaderMap::new(),
            json!({"project_id": "p", "token": "opt-token", "zen_api_key": "zen"}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        token_plan.headers.get(AUTHORIZATION).unwrap(),
        "Bearer opt-token"
    );
    assert_eq!(
        body(&token_plan),
        json!({"model_id": "granite-13b", "project_id": "p", "token": "opt-token"})
    );
}
