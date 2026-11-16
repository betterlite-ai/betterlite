use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::openai_credentials::Settings as OpenAiSettings;
use betterlite_openai::api::container_contracts::config::{
    ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::containers::OpenAiContainerConfig;
use betterlite_openai::containers::costs::ContainerCosts;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::evals::openai::OpenAiEvalsConfig;
use betterlite_openai::evals::transformation::EvalsRequestContext;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::openai_resource_dispatch::containers::{
    Call as ContainerCall, Dispatch as ContainerDispatch, Outcome as ContainerOutcome,
};
use betterlite_provider::openai_resource_dispatch::evals::{
    Call as EvalsCall, Dispatch as EvalsDispatch, IdCall, Outcome as EvalsOutcome,
};
use betterlite_provider::openai_resource_dispatch::vector_stores::{
    Call as VectorCall, Dispatch as VectorDispatch, Outcome as VectorOutcome,
    RequestCall as VectorRequest,
};
use betterlite_provider::openai_resource_dispatch::{
    OpenAiResourceOperation, OpenAiResourceResult, dispatch,
};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::json;

use super::{ContainersList, EvalsGet, OTHER_OUTCOME, VectorStoresCreate, failure, leaf_failure};

#[derive(Debug)]
struct ZeroCosts;

impl ContainerCosts for ZeroCosts {
    fn code_interpreter_cost(&self, _sessions: u32, _provider: &str) -> Decimal {
        Decimal::ZERO
    }
}

pub(super) async fn containers_list(case: &ContainersList) -> HelperResult {
    let config = OpenAiContainerConfig {
        settings: OpenAiSettings::default(),
        globals: LiteLlmGlobals::default(),
        env_api_key: None,
        costs: Arc::new(ZeroCosts),
    };
    let litellm_params = Object::new();
    let request_headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params: &litellm_params,
        headers: &request_headers,
    };
    let filter = ContainerListQuery {
        after: case.after.as_deref(),
        limit: case.limit,
        order: case.order.as_deref(),
        extra_query: case.extra_query.as_ref(),
    };
    let call = ContainerCall::List {
        api_base: &case.api_base,
        query: filter,
    };
    let operation = OpenAiResourceOperation::Containers(Box::new(ContainerDispatch {
        config: &config,
        context,
        call,
    }));
    let outcome = dispatch(operation).await.map_err(leaf_failure)?;
    let OpenAiResourceResult::Containers(ContainerOutcome::Request((url, query))) = outcome else {
        return Err(failure("Variant", OTHER_OUTCOME));
    };
    Ok(json!({ "url": url, "query": query }))
}

pub(super) async fn evals_get(case: &EvalsGet) -> HelperResult {
    let config = OpenAiEvalsConfig {
        api_key: Some(SecretString::from("sk-evals")),
        openai_key: None,
        env_api_key: None,
    };
    let litellm_params = Object::new();
    let request_headers = Headers::new();
    let context = EvalsRequestContext {
        litellm_params: &litellm_params,
        headers: &request_headers,
    };
    let call = EvalsCall::ById(IdCall::GetEval {
        eval_id: &case.eval_id,
        api_base: &case.api_base,
    });
    let operation = OpenAiResourceOperation::Evals(Box::new(EvalsDispatch {
        config: &config,
        context,
        call,
    }));
    let outcome = dispatch(operation).await.map_err(leaf_failure)?;
    let OpenAiResourceResult::Evals(EvalsOutcome::Headers((url, headers))) = outcome else {
        return Err(failure("Variant", OTHER_OUTCOME));
    };
    Ok(json!({ "url": url, "headers": headers }))
}

pub(super) async fn vector_stores_create(case: &VectorStoresCreate) -> HelperResult {
    let settings = OpenAiSettings {
        api_base: case.api_base.clone(),
        organization: None,
    };
    let call = VectorCall::Request(VectorRequest::Create {
        params: &case.params,
        api_base: &case.api_base,
    });
    let operation = OpenAiResourceOperation::VectorStores(Box::new(VectorDispatch {
        settings: &settings,
        globals: LiteLlmGlobals::default(),
        env_api_key: None,
        call,
    }));
    let outcome = dispatch(operation).await.map_err(leaf_failure)?;
    let OpenAiResourceResult::VectorStores(VectorOutcome::Request(request)) = outcome else {
        return Err(failure("Variant", OTHER_OUTCOME));
    };
    Ok(json!({ "url": request.url, "body": request.body }))
}
