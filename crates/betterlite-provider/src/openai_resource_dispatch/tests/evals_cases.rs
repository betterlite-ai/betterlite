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
use crate::openai_resource_dispatch::evals::{Call, Dispatch, IdCall, Outcome, PayloadCall, RunId};
use crate::openai_resource_dispatch::{
    OpenAiResourceError, OpenAiResourceOperation, OpenAiResourceResult, dispatch,
};

const API_BASE: &str = "https://api.evals.test";
const EVAL_URL: &str = "https://api.evals.test/v1/evals/eval_1";
const RUN_URL: &str = "https://api.evals.test/v1/evals/eval_1/runs/run_1";
const RUNS_URL: &str = "https://api.evals.test/v1/evals/eval_1/runs";

// What a transform returns: the url when it has one, whether the auth headers came back, and the body
struct Expected {
    url: Option<&'static str>,
    headers: bool,
    body: Option<Object>,
}

impl Expected {
    fn new(url: Option<&'static str>, headers: bool, body: Option<&str>) -> Self {
        Self {
            url,
            headers,
            body: body.map(object),
        }
    }
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
        Outcome::Url((url, body)) => (Some(url), None, Some(body)),
        Outcome::Headers((url, headers)) => (Some(url), Some(headers), None),
        Outcome::HeadersAndBody((url, headers, body)) => (Some(url), Some(headers), Some(body)),
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

fn get(eval_id: &'static str) -> IdCall<'static> {
    IdCall::GetEval {
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

async fn payload_rows() {
    let data = create_data();
    let created = r#"{"name":"smoke","data_source_config":{"type":"custom","item_schema":{"type":"object"},"include_sample_schema":true},"testing_criteria":[{"type":"llm_as_judge","model":"gpt-test"}]}"#;
    let create = Call::Payload(PayloadCall::CreateEval(&data));
    row(create, Expected::new(None, false, Some(created))).await;
    let listing = ListEvalsParams {
        limit: Some(3),
        after: Some(String::from("eval_0")),
        before: None,
        order: Some(EvalOrder::Desc),
        order_by: Some(EvalOrderBy::UpdatedAt),
    };
    let listed = r#"{"limit":3,"after":"eval_0","order":"desc","order_by":"updated_at"}"#;
    let list_url = Some("https://api.evals.test/v1/evals");
    let list = Call::Payload(PayloadCall::ListEvals(&listing));
    row(list, Expected::new(list_url, false, Some(listed))).await;
    let run_data = CreateRunRequest {
        data_source: object(r#"{"type":"completions"}"#),
        name: Some(String::from("nightly")),
        metadata: None,
    };
    let run_body = r#"{"data_source":{"type":"completions"},"name":"nightly"}"#;
    let create_run = Call::Payload(PayloadCall::CreateRun {
        eval_id: "eval_1",
        request: &run_data,
    });
    let expected = Expected::new(Some(RUNS_URL), false, Some(run_body));
    row(create_run, expected).await;
    let run_listing = ListRunsParams {
        limit: Some(2),
        after: None,
        before: Some(String::from("run_0")),
        order: Some(EvalOrder::Asc),
    };
    let run_listed = r#"{"limit":2,"before":"run_0","order":"asc"}"#;
    let list_runs = Call::Payload(PayloadCall::ListRuns {
        eval_id: "eval_1",
        params: &run_listing,
    });
    let expected = Expected::new(Some(RUNS_URL), false, Some(run_listed));
    row(list_runs, expected).await;
}

async fn eval_id_rows() {
    let fetch = Call::ById(get("eval_1"));
    row(fetch, Expected::new(Some(EVAL_URL), true, None)).await;
    let delete = Call::ById(IdCall::DeleteEval {
        eval_id: "eval_1",
        api_base: API_BASE,
    });
    row(delete, Expected::new(Some(EVAL_URL), true, None)).await;
    let update = UpdateEvalRequest {
        name: Some(String::from("renamed")),
        metadata: Some(object(r#"{"team":"core"}"#)),
    };
    let renamed = r#"{"name":"renamed","metadata":{"team":"core"}}"#;
    let change = Call::ById(IdCall::UpdateEval {
        eval_id: "eval_1",
        request: &update,
        api_base: API_BASE,
    });
    row(change, Expected::new(Some(EVAL_URL), true, Some(renamed))).await;
    let cancel = Call::ById(IdCall::CancelEval {
        eval_id: "eval_1",
        api_base: API_BASE,
    });
    let cancel_url = Some("https://api.evals.test/v1/evals/eval_1/cancel");
    row(cancel, Expected::new(cancel_url, true, Some("{}"))).await;
}

async fn run_id_rows() {
    let fetch = Call::ById(IdCall::GetRun(run_id()));
    row(fetch, Expected::new(Some(RUN_URL), true, None)).await;
    let delete = Call::ById(IdCall::DeleteRun(run_id()));
    row(delete, Expected::new(Some(RUN_URL), true, Some("{}"))).await;
    let cancel = Call::ById(IdCall::CancelRun(run_id()));
    let cancel_url = Some("https://api.evals.test/v1/evals/eval_1/runs/run_1/cancel");
    row(cancel, Expected::new(cancel_url, true, Some("{}"))).await;
}

async fn error_row() {
    let result = run(Call::ById(get(".."))).await;
    let Err(error) = result else {
        panic!("a dot eval id is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::Evals(EvalsError::PathSegment { .. })
    ));
}

pub(crate) async fn check() {
    payload_rows().await;
    eval_id_rows().await;
    run_id_rows().await;
    error_row().await;
}
