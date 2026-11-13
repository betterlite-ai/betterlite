use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::judge_primitives::judge_verdict_parsing::parse_json_verdict;
use betterlite_models::judge_primitives::provider_qualified::ProviderCache;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_server::judge_targets::call::JudgeCall;
use betterlite_server::judge_targets::dispatcher::JudgeDispatcher;
use betterlite_server::judge_targets::error::{JudgeCallError, JudgeDispatchError};
use betterlite_server::judge_targets::router::JudgeRouter;
use betterlite_server::judge_targets::sdk::JudgeSdk;
use betterlite_server::judge_targets::{judge_acompletion, judge_target};

use crate::helpers::HelperError;

use input::{DispatchInput, Input, QueryInput, ServedModels, TargetInput};

mod input;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Parse { raw } => {
                let verdict =
                    parse_json_verdict(raw).map_err(|error| helper_error("ValueError", error))?;
                serde_json::to_value(verdict).map_err(|error| helper_error("ValueError", error))
            }
            Input::Target(target) => run_target(target),
        }
    }
}

fn run_target(target: &TargetInput) -> crate::helpers::HelperResult {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let (known, environment, custom) = (BTreeSet::new(), BTreeMap::new(), BTreeSet::new());
    let context = ResolutionContext {
        catalog: catalog.as_ref(),
        known_model_names: &known,
        custom_providers: &custom,
        environment: &environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let cache = ProviderCache::default();
    let router_calls = Arc::new(Mutex::new(Vec::new()));
    let sdk_calls = Arc::new(Mutex::new(Vec::new()));
    let router = target.router.as_ref().map(|input| RecordingRouter {
        served: input.served.clone(),
        calls: Arc::clone(&router_calls),
    });
    let sdk = RecordingSdk {
        calls: Arc::clone(&sdk_calls),
    };
    let router_handle: Option<&dyn JudgeRouter> = router
        .as_ref()
        .map(|recording| -> &dyn JudgeRouter { recording });
    let dispatcher = JudgeDispatcher {
        router: router_handle,
        sdk: &sdk,
        cache: &cache,
        context: &context,
    };
    let mut result = serde_json::Map::new();
    let queries = query_values(&dispatcher, &target.queries);
    result.insert("queries".to_owned(), serde_json::Value::Array(queries));
    if let Some(dispatch) = &target.dispatch {
        let outcome = dispatch_value(&dispatcher, dispatch, &router_calls, &sdk_calls)?;
        result.insert("dispatch".to_owned(), outcome);
    }
    Ok(serde_json::Value::Object(result))
}

fn query_values(
    dispatcher: &JudgeDispatcher<'_>,
    queries: &[QueryInput],
) -> Vec<serde_json::Value> {
    queries
        .iter()
        .map(|query| query_value(dispatcher, query))
        .collect()
}

fn query_value(dispatcher: &JudgeDispatcher<'_>, query: &QueryInput) -> serde_json::Value {
    let target = judge_target(dispatcher, &query.model, query.team_id.as_deref());
    serde_json::json!({
        "via": target.via,
        "models": target.models.into_iter().collect::<Vec<String>>(),
    })
}

fn dispatch_value(
    dispatcher: &JudgeDispatcher<'_>,
    dispatch: &DispatchInput,
    router_calls: &Arc<Mutex<Vec<JudgeCall>>>,
    sdk_calls: &Arc<Mutex<Vec<JudgeCall>>>,
) -> crate::helpers::HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error("RuntimeError", error))?;
    let outcome = runtime.block_on(judge_acompletion(
        dispatcher,
        &dispatch.model,
        dispatch.messages.clone(),
        dispatch.team_id.as_deref(),
        dispatch.params.clone(),
    ));
    match outcome {
        Ok(_) => recorded(router_calls).map_or_else(
            || {
                recorded(sdk_calls).map_or_else(
                    || Ok(serde_json::json!({"declined": true})),
                    |sdk_call| call_value("sdk", &sdk_call),
                )
            },
            |router_call| call_value("router", &router_call),
        ),
        Err(JudgeCallError::Nothing { model }) => Ok(serde_json::json!({"declined": model})),
        Err(JudgeCallError::Dispatch { model, source }) => {
            Ok(serde_json::json!({"failed": model, "message": source.message}))
        }
    }
}

fn recorded(calls: &Arc<Mutex<Vec<JudgeCall>>>) -> Option<JudgeCall> {
    calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .cloned()
}

fn call_value(kind: &str, call: &JudgeCall) -> crate::helpers::HelperResult {
    let mut fields = serde_json::Map::new();
    fields.insert(
        "kind".to_owned(),
        serde_json::Value::String(kind.to_owned()),
    );
    fields.insert(
        "model".to_owned(),
        serde_json::Value::String(call.model.clone()),
    );
    fields.insert(
        "messages".to_owned(),
        serde_json::to_value(&call.messages).map_err(|error| helper_error("ValueError", error))?,
    );
    fields.insert(
        "num_retries".to_owned(),
        serde_json::Value::from(call.num_retries),
    );
    if let Some(fallbacks) = &call.fallbacks {
        fields.insert(
            "fallbacks".to_owned(),
            serde_json::to_value(fallbacks).map_err(|error| helper_error("ValueError", error))?,
        );
    }
    fields.insert(
        "drop_params".to_owned(),
        serde_json::Value::Bool(call.drop_params),
    );
    let params = call
        .params
        .iter()
        .map(|(name, param)| {
            serde_json::to_value(param)
                .map(|converted| (name.clone(), converted))
                .map_err(|error| helper_error("ValueError", error))
        })
        .collect::<Result<serde_json::Map<String, serde_json::Value>, _>>()?;
    fields.extend(params);
    Ok(serde_json::Value::Object(fields))
}

struct RecordingRouter {
    served: Vec<ServedModels>,
    calls: Arc<Mutex<Vec<JudgeCall>>>,
}

#[async_trait]
impl JudgeRouter for RecordingRouter {
    fn resolved_litellm_models(&self, model: &str, team_id: Option<&str>) -> Vec<String> {
        self.served
            .iter()
            .filter(|entry| {
                entry.model == model
                    && entry
                        .team_id
                        .as_deref()
                        .is_none_or(|team| Some(team) == team_id)
            })
            .flat_map(|entry| entry.models.iter().cloned())
            .collect()
    }

    async fn acompletion(&self, call: JudgeCall) -> Result<Json, JudgeDispatchError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        Ok(Json::Null)
    }
}

struct RecordingSdk {
    calls: Arc<Mutex<Vec<JudgeCall>>>,
}

#[async_trait]
impl JudgeSdk for RecordingSdk {
    async fn acompletion(&self, call: JudgeCall) -> Result<Json, JudgeDispatchError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        Ok(Json::Null)
    }
}

fn helper_error<Message>(class: &str, message: Message) -> Box<HelperError>
where
    Message: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
