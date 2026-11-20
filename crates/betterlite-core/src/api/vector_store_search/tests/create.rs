use rstest::rstest;

use super::*;
use crate::api::vector_store_http_write::service::LogContext;
use crate::api::vector_store_search::service::{CreateCall, create};

impl Logs {
    pub(super) async fn run_create<S>(
        &mut self,
        deps: &SdkDeps<'_, S>,
        call: &CreateCall<'_>,
    ) -> Result<Json, VectorStoreSearchError>
    where
        S: ClientSource,
    {
        let env = pre_post_env(&self.redactor, &self.sink, self.now);
        create(
            deps,
            call,
            &mut LogContext {
                state: &mut self.state,
                env: &env,
            },
        )
        .await
    }
}

pub(super) fn create_call<'call>(
    optional: &'call Params,
    kwargs: &'call Params,
    entry: Entry,
) -> CreateCall<'call> {
    CreateCall {
        optional,
        kwargs,
        extra_headers: None,
        extra_body: None,
        timeout: None,
        custom_llm_provider: None,
        entry,
    }
}

#[rstest]
#[case::sync(Entry::Sync)]
#[case::asynchronous(Entry::Asynchronous)]
#[tokio::test]
async fn create_optional_fields(#[case] entry: Entry) {
    let fakes = Fakes::new();
    let source = fakes.source(CREATE_RESULT);
    let mut logs = Logs::new("vector_store_create");
    let optional = params(CREATE_OPTIONALS);
    let kwargs = params(r#"{"api_key":"sk-client","api_base":"https://mock.invalid/v1"}"#);
    let call = create_call(&optional, &kwargs, entry);
    let deps = fakes.deps(&source, None);
    let result = logs.run_create(&deps, &call).await;
    assert_eq!(
        result.expect("the mock provider answers"),
        json(CREATE_RESULT)
    );
    fakes.seen.with(|seen| {
        assert_eq!(
            seen.bodies.last().expect("the create posted"),
            &json(CREATE_OPTIONALS)
        );
        assert_eq!(seen.create_api_keys, vec![Some(String::from("sk-client"))]);
    });
    assert_eq!(logs.detail("optional_params"), json(CREATE_OPTIONALS));
    assert_eq!(
        logs.detail("custom_llm_provider"),
        Json::Text(String::from("openai"))
    );
    assert_eq!(
        logs.detail("model"),
        Json::Text(String::from("vector-store"))
    );
    let Json::Object(litellm_params) = logs.detail("litellm_params") else {
        panic!("the logged litellm params are an object");
    };
    assert_eq!(litellm_params.get("litellm_call_id"), Some(&Json::Null));
}

// an empty create dict proceeds to HTTP, and a nonempty one comes back unchanged
#[rstest]
#[case::empty_posted(r#"{"api_base":"https://mock.invalid/v1","mock_response":{}}"#, true)]
#[case::nonempty_returned(
    r#"{"api_base":"https://mock.invalid/v1","mock_response":{"id":"vs_a","object":"vector_store"}}"#,
    false
)]
#[tokio::test]
async fn vector_mock_create_truthiness(#[case] mock: &str, #[case] posted: bool) {
    let fakes = Fakes::new();
    let source = fakes.source(CREATE_RESULT);
    let mut logs = Logs::new("vector_store_create");
    let optional = Params::new();
    let kwargs = params(mock);
    let call = create_call(&optional, &kwargs, Entry::Sync);
    let deps = fakes.deps(&source, None);
    let before = fakes.seen.with(|seen| seen.clients.len());
    let result = logs
        .run_create(&deps, &call)
        .await
        .expect("the create answers");
    assert_eq!(result, json(CREATE_RESULT));
    let after = fakes.seen.with(|seen| seen.clients.len());
    assert_eq!(after, before + usize::from(posted));
}

// a create with no optionals sends the five declared fields as nulls
#[tokio::test]
async fn create_sends_the_declared_fields() {
    let fakes = Fakes::new();
    let source = fakes.source(CREATE_RESULT);
    let mut logs = Logs::new("vector_store_create");
    let optional = Params::new();
    let kwargs = params(r#"{"api_base":"https://mock.invalid/v1"}"#);
    let mut headers = BTreeMap::new();
    headers.insert("X-Caller".to_owned(), "yes".to_owned());
    let mut call = create_call(&optional, &kwargs, Entry::Sync);
    call.extra_headers = Some(&headers);
    let deps = fakes.deps(&source, None);
    logs.run_create(&deps, &call)
        .await
        .expect("the mock provider answers");
    fakes.seen.with(|seen| {
        assert_eq!(
            seen.bodies.last().expect("the create posted"),
            &json(
                r#"{"name":null,"file_ids":null,"expires_after":null,"chunking_strategy":null,"metadata":null}"#
            )
        );
        let sent = seen.requests.last().expect("the request was recorded");
        assert_eq!(
            sent.headers
                .get("X-Caller")
                .map(|value| value.to_str().expect("the header is text")),
            Some("yes")
        );
    });
}
