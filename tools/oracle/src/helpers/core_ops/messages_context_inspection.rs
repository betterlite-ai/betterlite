use async_trait::async_trait;
// ast-grep-ignore: no-long-names
use betterlite_core::api::messages_chat_prepare::inspect::inspect_compaction_and_proxy_context;
use betterlite_core::api::messages_chat_prepare::prediction::predict_context_edit_execution;
use betterlite_core::api::summary_execution::completion::{
    SummaryCall, SummaryCallFailure, SummaryCompletion,
};
use betterlite_core::api::summary_execution::extract::CompletionAnswer;
use betterlite_models::decisions::{Extra, Json};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};

// the value both sides report for a proxy handle the request carries
const ROUTER_AVAILABLE: &str = "injected-router";

// the pure functions a /v1/messages request reaches before any history or edit runs
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    inspections: Vec<InspectionInput>,
    #[serde(default)]
    predictions: Vec<PredictionInput>,
}

// what the proxy module holds for llm_router: the handle, an empty slot, or no module attribute at all
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RouterMode {
    Handle,
    Empty,
    Missing,
}

// one request's block verdict, kwargs and proxy handle
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectionInput {
    messages: Vec<Json>,
    kwargs: Extra,
    router: RouterMode,
}

// one spec plus the caller's additional_drop_params
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PredictionInput {
    #[serde(default)]
    context_management_spec: Option<Json>,
    #[serde(default)]
    additional_drop_params: Option<Json>,
}

// the proxy's injected llm_router: an inspection only reports that the wrapper has one
struct ProxyRouter;

#[async_trait]
impl SummaryCompletion for ProxyRouter {
    async fn complete(
        &self,
        _call: SummaryCall<'_>,
    ) -> Result<CompletionAnswer, SummaryCallFailure> {
        Err(SummaryCallFailure {
            message: String::from("an inspection never calls the router"),
        })
    }
}

#[derive(Debug, Serialize)]
struct Report {
    inspections: Vec<Inspection>,
    predictions: Vec<Prediction>,
}

#[derive(Debug, Serialize)]
struct Inspection {
    has_compaction_block: bool,
    litellm_metadata: Option<Object>,
    user_api_key_auth: Option<Json>,
    router: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct Prediction {
    compact: bool,
    non_compact: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let report = Report {
            inspections: input.inspections.iter().map(inspect).collect(),
            predictions: input.predictions.iter().map(predict).collect(),
        };
        serde_json::to_value(report).or_value_error()
    }
}

fn inspect(input: &InspectionInput) -> Inspection {
    let handle = ProxyRouter;
    let router =
        matches!(input.router, RouterMode::Handle).then_some(&handle as &dyn SummaryCompletion);
    // ast-grep-ignore: no-long-names
    let found = inspect_compaction_and_proxy_context(&input.messages, &input.kwargs, router);
    let (litellm_metadata, user_api_key_auth) = match found.proxy {
        Some(proxy) => (Some(proxy.litellm_metadata), proxy.user_api_key_auth),
        None => (None, None),
    };
    Inspection {
        has_compaction_block: found.has_compaction_block,
        litellm_metadata,
        user_api_key_auth,
        router: found.router.map(|_handle| ROUTER_AVAILABLE),
    }
}

fn predict(input: &PredictionInput) -> Prediction {
    let predicted = predict_context_edit_execution(
        input.context_management_spec.as_ref(),
        input.additional_drop_params.as_ref(),
    );
    Prediction {
        compact: predicted.compact,
        non_compact: predicted.non_compact,
    }
}
