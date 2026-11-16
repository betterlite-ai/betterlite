use std::collections::BTreeMap;

use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::transformation::EvalsRequestContext;
use betterlite_openai::evals::types::{
    CreateEvalRequest, CreateRunRequest, DataSourceConfig, DataSourceConfigCustom, EvalOrder,
    EvalOrderBy, GraderConfig, LLMAsJudgeGraderConfig, ListEvalsParams, ListRunsParams,
    UpdateEvalRequest,
};
use betterlite_openai::openai_base_config::{Headers, Object};

use super::fixture::{evals_config, object};
use crate::openai_resource_dispatch::evals::{Call, Dispatch, EvalCall, Outcome, RunCall, RunId};
use crate::openai_resource_dispatch::{
    OpenAiResourceError, OpenAiResourceOperation, OpenAiResourceResult, dispatch,
};

const API_BASE: &str = "https://api.evals.test";
const EVAL_URL: &str = "https://api.evals.test/v1/evals/eval_1";
const RUN_URL: &str = "https://api.evals.test/v1/evals/eval_1/runs/run_1";

// What a transform returns: the url when it has one, whether the auth headers came back, and the body
struct Expected {
    url: Option<&'static str>,
    headers: bool,
    body: Option<Object>,
}

fn auth_headers() -> Headers {
    BTreeMap::from([
        (
            String::from("Authorization"),
            String::from("Bearer sk-evals"),
        ),
        (
            String::from("Content-Type"),
            String::from("application/json"),
        ),
    ])
}

async fn run(call: Call<'_>) -> Result<OpenAiResourceResult, OpenAiResourceError> {
    let config = evals_config();
    let params = object(r#"{"api_base":"https://api.evals.test"}"#);
    let sent = auth_headers();
    let context = EvalsRequestContext {
        litellm_params: &params,
        headers: &sent,
    };
    let operation = OpenAiResourceOperation::Evals(Box::new(Dispatch {
        config: &config,
        context,
        call,
    }));
    dispatch(operation).await
}

async fn row(call: Call<'_>, expected: Expected) {
    let result = run(call).await;
    let Ok(OpenAiResourceResult::Evals(outcome)) = result else {
        panic!("the transform returns an outcome");
    };
    let (url, returned, body) = match outcome {
        Outcome::Body(body) => (None, None, Some(body)),
        Outcome::Url { url, body } => (Some(url), None, Some(body)),
        Outcome::Headers { url, headers } => (Some(url), Some(headers), None),
        Outcome::HeadersAndBody { url, headers, body } => (Some(url), Some(headers), Some(body)),
    };
    assert_eq!(url.as_deref(), expected.url);
    assert_eq!(returned, expected.headers.then(auth_headers));
    assert_eq!(body, expected.body);
}

fn create_data() -> CreateEvalRequest {
    CreateEvalRequest {
        name: Some(String::from("smoke")),
        data_source_config: DataSourceConfig::Custom(DataSourceConfigCustom {
            item_schema: object(r#"{"type":"object"}"#),
            include_sample_schema: Some(true),
        }),
        testing_criteria: vec![GraderConfig::LlmAsJudge(LLMAsJudgeGraderConfig {
            model: Some(String::from("gpt-test")),
            prompt: None,
        })],
        metadata: None,
    }
}

fn get(eval_id: &'static str) -> EvalCall<'static> {
    EvalCall::Get {
        eval_id,
        api_base: API_BASE,
    }
}

fn run_id() -> RunId<'static> {
    RunId {
        eval_id: "eval_1",
        run_id: "run_1",
        api_base: API_BASE,
    }
}

async fn eval_rows() {
    let data = create_data();
    let created = r#"{"name":"smoke","data_source_config":{"type":"custom","item_schema":{"type":"object"},"include_sample_schema":true},"testing_criteria":[{"type":"llm_as_judge","model":"gpt-test"}]}"#;
    let expected = Expected {
        url: None,
        headers: false,
        body: Some(object(created)),
    };
    row(Call::Eval(EvalCall::Create(&data)), expected).await;
    let listing = ListEvalsParams {
        limit: Some(3),
        after: Some(String::from("eval_0")),
        before: None,
        order: Some(EvalOrder::Desc),
        order_by: Some(EvalOrderBy::UpdatedAt),
    };
    let listed = r#"{"limit":3,"after":"eval_0","order":"desc","order_by":"updated_at"}"#;
    let expected = Expected {
        url: Some("https://api.evals.test/v1/evals"),
        headers: false,
        body: Some(object(listed)),
    };
    row(Call::Eval(EvalCall::List(&listing)), expected).await;
    let expected = Expected {
        url: Some(EVAL_URL),
        headers: true,
        body: None,
    };
    row(Call::Eval(get("eval_1")), expected).await;
    let delete = EvalCall::Delete {
        eval_id: "eval_1",
        api_base: API_BASE,
    };
    let expected = Expected {
        url: Some(EVAL_URL),
        headers: true,
        body: None,
    };
    row(Call::Eval(delete), expected).await;
}

async fn eval_body_rows() {
    let update = UpdateEvalRequest {
        name: Some(String::from("renamed")),
        metadata: Some(object(r#"{"team":"core"}"#)),
    };
    let call = EvalCall::Update {
        eval_id: "eval_1",
        request: &update,
        api_base: API_BASE,
    };
    let expected = Expected {
        url: Some(EVAL_URL),
        headers: true,
        body: Some(object(r#"{"name":"renamed","metadata":{"team":"core"}}"#)),
    };
    row(Call::Eval(call), expected).await;
    let cancel = EvalCall::Cancel {
        eval_id: "eval_1",
        api_base: API_BASE,
    };
    let expected = Expected {
        url: Some("https://api.evals.test/v1/evals/eval_1/cancel"),
        headers: true,
        body: Some(object("{}")),
    };
    row(Call::Eval(cancel), expected).await;
}

async fn run_rows() {
    let data = CreateRunRequest {
        data_source: object(r#"{"type":"completions"}"#),
        name: Some(String::from("nightly")),
        metadata: None,
    };
    let create = RunCall::Create {
        eval_id: "eval_1",
        request: &data,
    };
    let runs_url = "https://api.evals.test/v1/evals/eval_1/runs";
    let expected = Expected {
        url: Some(runs_url),
        headers: false,
        body: Some(object(
            r#"{"data_source":{"type":"completions"},"name":"nightly"}"#,
        )),
    };
    row(Call::Run(create), expected).await;
    let listing = ListRunsParams {
        limit: Some(2),
        after: None,
        before: Some(String::from("run_0")),
        order: Some(EvalOrder::Asc),
    };
    let list = RunCall::List {
        eval_id: "eval_1",
        params: &listing,
    };
    let expected = Expected {
        url: Some(runs_url),
        headers: false,
        body: Some(object(r#"{"limit":2,"before":"run_0","order":"asc"}"#)),
    };
    row(Call::Run(list), expected).await;
}

async fn run_path_rows() {
    let expected = Expected {
        url: Some(RUN_URL),
        headers: true,
        body: None,
    };
    row(Call::Run(RunCall::Get(run_id())), expected).await;
    let expected = Expected {
        url: Some(RUN_URL),
        headers: true,
        body: Some(object("{}")),
    };
    row(Call::Run(RunCall::Delete(run_id())), expected).await;
    let expected = Expected {
        url: Some("https://api.evals.test/v1/evals/eval_1/runs/run_1/cancel"),
        headers: true,
        body: Some(object("{}")),
    };
    row(Call::Run(RunCall::Cancel(run_id())), expected).await;
}

async fn error_row() {
    let result = run(Call::Eval(get(".."))).await;
    let Err(error) = result else {
        panic!("a dot eval id is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::Evals(EvalsError::PathSegment { .. })
    ));
}

pub(crate) async fn check() {
    eval_rows().await;
    eval_body_rows().await;
    run_rows().await;
    run_path_rows().await;
    error_row().await;
}
