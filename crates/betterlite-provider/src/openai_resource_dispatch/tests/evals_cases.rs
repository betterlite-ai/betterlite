use std::collections::BTreeMap;

use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::transformation::{
    BaseEvalsAPIConfig, EvalsHttpResponse, EvalsRequestContext,
};
use betterlite_openai::evals::types::{
    CreateEvalRequest, CreateRunRequest, DataSourceConfig, DataSourceConfigCustom, EvalOrder,
    EvalOrderBy, GraderConfig, LLMAsJudgeGraderConfig, ListEvalsParams, ListRunsParams,
    UpdateEvalRequest,
};
use betterlite_openai::openai_base_config::{Headers, Object};

use super::fixture::{evals_config, object};
use crate::openai_resource_dispatch::evals::{
    Call, Dispatch, EvalCall, EvalReply, Outcome, ReplyCall, RunCall, RunReply,
};
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

fn reply<'a>(text: &'a str, headers: &'a Headers) -> EvalsHttpResponse<'a> {
    EvalsHttpResponse {
        status_code: 200,
        headers,
        text,
    }
}

fn eval_body(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","object":"eval","created_at":1700000000,"name":"smoke","data_source_config":{{"type":"custom"}},"testing_criteria":[]}}"#
    )
}

async fn eval_requests() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let data = create_data();
    let created = run(
        &config,
        context,
        Call::Eval(EvalCall::Create { request: &data }),
    )
    .await;
    let Ok(OpenAiResourceResult::Evals(Outcome::Body(body))) = created else {
        panic!("the create eval transform returns its body");
    };
    assert_eq!(
        body,
        object(
            r#"{"name":"smoke","data_source_config":{"type":"custom","item_schema":{"type":"object"},"include_sample_schema":true},"testing_criteria":[{"type":"llm_as_judge","model":"gpt-test"}]}"#
        )
    );
    let listed = run(
        &config,
        context,
        Call::Eval(EvalCall::List {
            params: &ListEvalsParams {
                limit: Some(3),
                after: Some(String::from("eval_0")),
                before: None,
                order: Some(EvalOrder::Desc),
                order_by: Some(EvalOrderBy::UpdatedAt),
            },
        }),
    )
    .await;
    let Ok(OpenAiResourceResult::Evals(Outcome::Url { url, body })) = listed else {
        panic!("the list evals transform returns its url and body");
    };
    assert_eq!(url, "https://api.evals.test/v1/evals");
    assert_eq!(
        body,
        object(r#"{"limit":3,"after":"eval_0","order":"desc","order_by":"updated_at"}"#)
    );
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

async fn eval_mutations() {
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

async fn run_requests() {
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
    let listed = run(
        &config,
        context,
        Call::Run(RunCall::List {
            eval_id: "eval_1",
            params: &ListRunsParams {
                limit: Some(2),
                after: None,
                before: Some(String::from("run_0")),
                order: Some(EvalOrder::Asc),
            },
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

async fn run_paths() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let fetched = run(
        &config,
        context,
        Call::Run(RunCall::Get {
            eval_id: "eval_1",
            run_id: "run_1",
            api_base: API_BASE,
        }),
    )
    .await;
    let Ok(OpenAiResourceResult::Evals(Outcome::Headers { url, headers: sent })) = fetched else {
        panic!("the get run transform returns its url and headers");
    };
    assert_eq!(url, "https://api.evals.test/v1/evals/eval_1/runs/run_1");
    assert_eq!(sent, headers);
    let cancelled = run(
        &config,
        context,
        Call::Run(RunCall::Cancel {
            eval_id: "eval_1",
            run_id: "run_1",
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
        panic!("the cancel run transform returns its url, headers and body");
    };
    assert_eq!(
        url,
        "https://api.evals.test/v1/evals/eval_1/runs/run_1/cancel"
    );
    assert_eq!(sent, headers);
    assert_eq!(body, object("{}"));
    let deleted = run(
        &config,
        context,
        Call::Run(RunCall::Delete {
            eval_id: "eval_1",
            run_id: "run_1",
            api_base: API_BASE,
        }),
    )
    .await;
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

async fn eval_responses() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let body = eval_body("eval_reply");
    let created = run(
        &config,
        context,
        Call::Reply(ReplyCall::Eval(EvalReply::Create(reply(&body, &headers)))),
    )
    .await;
    let Ok(OpenAiResourceResult::Evals(Outcome::Eval(eval))) = created else {
        panic!("the create eval reply decodes to an eval");
    };
    assert_eq!(eval.id, "eval_reply");
    let listed_body = format!(
        r#"{{"object":"list","data":[{}],"first_id":"eval_reply","last_id":"eval_reply","has_more":false}}"#,
        eval_body("eval_reply")
    );
    let listed = run(
        &config,
        context,
        Call::Reply(ReplyCall::Eval(EvalReply::List(reply(
            &listed_body,
            &headers,
        )))),
    )
    .await;
    let Ok(OpenAiResourceResult::Evals(Outcome::Evals(page))) = listed else {
        panic!("the list evals reply decodes to a page");
    };
    assert_eq!(page.data[0].id, "eval_reply");
    let deleted = run(
        &config,
        context,
        Call::Reply(ReplyCall::Eval(EvalReply::Delete(reply(
            r#"{"eval_id":"eval_1","object":"eval.deleted","deleted":true}"#,
            &headers,
        )))),
    )
    .await;
    let Ok(OpenAiResourceResult::Evals(Outcome::EvalDeleted(deleted))) = deleted else {
        panic!("the delete eval reply decodes to a deleted eval");
    };
    assert!(deleted.deleted);
    assert_eq!(deleted.eval_id, "eval_1");
}

async fn run_responses() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let body = r#"{"id":"run_reply","object":"eval.run","created_at":1700000000,"status":"cancelled","data_source":{"type":"completions"},"eval_id":"eval_1"}"#;
    let cancelled = run(
        &config,
        context,
        Call::Reply(ReplyCall::Run(RunReply::Cancel(reply(body, &headers)))),
    )
    .await;
    let Ok(OpenAiResourceResult::Evals(Outcome::RunCancelled(cancelled))) = cancelled else {
        panic!("the cancel run reply decodes to a cancelled run");
    };
    assert_eq!(cancelled.id, "run_reply");
    let deleted = run(
        &config,
        context,
        Call::Reply(ReplyCall::Run(RunReply::Delete(reply(
            r#"{"run_id":"run_1","deleted":true}"#,
            &headers,
        )))),
    )
    .await;
    let Ok(OpenAiResourceResult::Evals(Outcome::RunDeleted(deleted))) = deleted else {
        panic!("the delete run reply decodes to a deleted run");
    };
    assert_eq!(deleted.run_id, "run_1");
}

async fn error_row() {
    let config = evals_config();
    let params = params();
    let headers = headers();
    let context = context(&params, &headers);
    let malformed = reply("not an eval", &headers);
    let result = run(
        &config,
        context,
        Call::Reply(ReplyCall::Eval(EvalReply::Create(malformed))),
    )
    .await;
    let Err(error) = result else {
        panic!("a malformed eval reply is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::Evals(EvalsError::ReplyJson { .. })
    ));
}

pub(crate) async fn check() {
    eval_requests().await;
    eval_mutations().await;
    run_requests().await;
    run_paths().await;
    eval_responses().await;
    run_responses().await;
    error_row().await;
}
