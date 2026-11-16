use std::collections::BTreeMap;

use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::transformation::{BaseEvalsAPIConfig, EvalsRequestContext};
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

fn headers() -> Headers {
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

fn params() -> Object {
    object(r#"{"api_base":"https://api.evals.test"}"#)
}

fn context<'a>(params: &'a Object, headers: &'a Headers) -> EvalsRequestContext<'a> {
    EvalsRequestContext {
        litellm_params: params,
        headers,
    }
}

async fn run(
    config: &dyn BaseEvalsAPIConfig<Error = EvalsError>,
    context: EvalsRequestContext<'_>,
    call: Call<'_>,
) -> Result<OpenAiResourceResult, OpenAiResourceError> {
    let operation = OpenAiResourceOperation::Evals(Box::new(Dispatch {
        config,
        context,
        call,
    }));
    dispatch(operation).await
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

async fn create_eval_row() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let data = create_data();
    let created = run(&config, context, Call::Eval(EvalCall::Create(&data))).await;
    let Ok(OpenAiResourceResult::Evals(Outcome::Body(body))) = created else {
        panic!("the create eval transform returns its body");
    };
    assert_eq!(
        body,
        object(
            r#"{"name":"smoke","data_source_config":{"type":"custom","item_schema":{"type":"object"},"include_sample_schema":true},"testing_criteria":[{"type":"llm_as_judge","model":"gpt-test"}]}"#
        )
    );
}

async fn list_evals_row() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let list_params = ListEvalsParams {
        limit: Some(3),
        after: Some(String::from("eval_0")),
        before: None,
        order: Some(EvalOrder::Desc),
        order_by: Some(EvalOrderBy::UpdatedAt),
    };
    let listed = run(&config, context, Call::Eval(EvalCall::List(&list_params))).await;
    let Ok(OpenAiResourceResult::Evals(Outcome::Url { url, body })) = listed else {
        panic!("the list evals transform returns its url and body");
    };
    assert_eq!(url, "https://api.evals.test/v1/evals");
    assert_eq!(
        body,
        object(r#"{"limit":3,"after":"eval_0","order":"desc","order_by":"updated_at"}"#)
    );
}

async fn get_eval_row() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let fetched = run(
        &config,
        context,
        Call::Eval(EvalCall::Get {
            eval_id: "eval_1",
            api_base: API_BASE,
        }),
    )
    .await;
    let Ok(OpenAiResourceResult::Evals(Outcome::Headers { url, headers: sent })) = fetched else {
        panic!("the get eval transform returns its url and headers");
    };
    assert_eq!(url, "https://api.evals.test/v1/evals/eval_1");
    assert_eq!(sent, headers);
}

async fn update_eval_row() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let update = UpdateEvalRequest {
        name: Some(String::from("renamed")),
        metadata: Some(object(r#"{"team":"core"}"#)),
    };
    let updated = run(
        &config,
        context,
        Call::Eval(EvalCall::Update {
            eval_id: "eval_1",
            request: &update,
            api_base: API_BASE,
        }),
    )
    .await;
    let Ok(OpenAiResourceResult::Evals(Outcome::HeadersAndBody {
        url,
        headers: sent,
        body,
    })) = updated
    else {
        panic!("the update eval transform returns its url, headers and body");
    };
    assert_eq!(url, "https://api.evals.test/v1/evals/eval_1");
    assert_eq!(sent, headers);
    assert_eq!(
        body,
        object(r#"{"name":"renamed","metadata":{"team":"core"}}"#)
    );
}

async fn delete_eval_row() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let deleted = run(
        &config,
        context,
        Call::Eval(EvalCall::Delete {
            eval_id: "eval_1",
            api_base: API_BASE,
        }),
    )
    .await;
    let Ok(OpenAiResourceResult::Evals(Outcome::Headers { url, headers: sent })) = deleted else {
        panic!("the delete eval transform returns its url and headers");
    };
    assert_eq!(url, "https://api.evals.test/v1/evals/eval_1");
    assert_eq!(sent, headers);
}

async fn cancel_eval_row() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let cancelled = run(
        &config,
        context,
        Call::Eval(EvalCall::Cancel {
            eval_id: "eval_1",
            api_base: API_BASE,
        }),
    )
    .await;
    let Ok(OpenAiResourceResult::Evals(Outcome::HeadersAndBody {
        url,
        headers: sent,
        body,
    })) = cancelled
    else {
        panic!("the cancel eval transform returns its url, headers and body");
    };
    assert_eq!(url, "https://api.evals.test/v1/evals/eval_1/cancel");
    assert_eq!(sent, headers);
    assert_eq!(body, object("{}"));
}

async fn create_run_row() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let data = CreateRunRequest {
        data_source: object(r#"{"type":"completions"}"#),
        name: Some(String::from("nightly")),
        metadata: None,
    };
    let created = run(
        &config,
        context,
        Call::Run(RunCall::Create {
            eval_id: "eval_1",
            request: &data,
        }),
    )
    .await;
    let Ok(OpenAiResourceResult::Evals(Outcome::Url { url, body })) = created else {
        panic!("the create run transform returns its url and body");
    };
    assert_eq!(url, "https://api.evals.test/v1/evals/eval_1/runs");
    assert_eq!(
        body,
        object(r#"{"data_source":{"type":"completions"},"name":"nightly"}"#)
    );
}

async fn list_runs_row() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let list_params = ListRunsParams {
        limit: Some(2),
        after: None,
        before: Some(String::from("run_0")),
        order: Some(EvalOrder::Asc),
    };
    let listed = run(
        &config,
        context,
        Call::Run(RunCall::List {
            eval_id: "eval_1",
            params: &list_params,
        }),
    )
    .await;
    let Ok(OpenAiResourceResult::Evals(Outcome::Url { url, body })) = listed else {
        panic!("the list runs transform returns its url and body");
    };
    assert_eq!(url, "https://api.evals.test/v1/evals/eval_1/runs");
    assert_eq!(
        body,
        object(r#"{"limit":2,"before":"run_0","order":"asc"}"#)
    );
}

fn run_id() -> RunId<'static> {
    RunId {
        eval_id: "eval_1",
        run_id: "run_1",
        api_base: API_BASE,
    }
}

async fn run_paths() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let fetched = run(&config, context, Call::Run(RunCall::Get(run_id()))).await;
    let Ok(OpenAiResourceResult::Evals(Outcome::Headers { url, headers: sent })) = fetched else {
        panic!("the get run transform returns its url and headers");
    };
    assert_eq!(url, "https://api.evals.test/v1/evals/eval_1/runs/run_1");
    assert_eq!(sent, headers);
    let deleted = run(&config, context, Call::Run(RunCall::Delete(run_id()))).await;
    let Ok(OpenAiResourceResult::Evals(Outcome::HeadersAndBody {
        url,
        headers: sent,
        body,
    })) = deleted
    else {
        panic!("the delete run transform returns its url, headers and body");
    };
    assert_eq!(url, "https://api.evals.test/v1/evals/eval_1/runs/run_1");
    assert_eq!(sent, headers);
    assert_eq!(body, object("{}"));
}

async fn cancelled_run() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let cancelled = run(&config, context, Call::Run(RunCall::Cancel(run_id()))).await;
    let Ok(OpenAiResourceResult::Evals(Outcome::HeadersAndBody {
        url,
        headers: sent,
        body,
    })) = cancelled
    else {
        panic!("the cancel run transform returns its url, headers and body");
    };
    assert_eq!(
        url,
        "https://api.evals.test/v1/evals/eval_1/runs/run_1/cancel"
    );
    assert_eq!(sent, headers);
    assert_eq!(body, object("{}"));
}

async fn error_row() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let result = run(
        &config,
        context,
        Call::Eval(EvalCall::Get {
            eval_id: "..",
            api_base: API_BASE,
        }),
    )
    .await;
    let Err(error) = result else {
        panic!("a dot eval id is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::Evals(EvalsError::PathSegment { .. })
    ));
}

pub(crate) async fn check() {
    create_eval_row().await;
    list_evals_row().await;
    get_eval_row().await;
    update_eval_row().await;
    delete_eval_row().await;
    cancel_eval_row().await;
    create_run_row().await;
    list_runs_row().await;
    run_paths().await;
    cancelled_run().await;
    error_row().await;
}
