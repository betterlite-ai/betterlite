use std::future::Future;
use std::pin::Pin;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::identity::declared::authenticating_provider;
use betterlite_models::resolution::ResolutionRequest;
use betterlite_openai::api::rerank_contracts::records::{RerankDocument, RerankResponse};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use crate::api::rerank_provider_plan::call::{RerankCall, ReturnDocuments};
use crate::api::rerank_provider_plan::dispatch::{
    RerankOperations, provider_failure, value_failure,
};
use crate::api::rerank_provider_plan::plan::{RerankException, RerankIdentity, RerankPlan, rerank};

mod tests;

// arerank's eight declared arguments and the kwargs the sync callee receives after them
#[derive(Debug, Clone)]
pub struct AsyncRerankArgs {
    pub model: String,
    pub query: String,
    pub documents: Vec<RerankDocument>,
    pub custom_llm_provider: Option<String>,
    pub top_n: Option<i64>,
    pub rank_fields: Option<Vec<String>>,
    pub return_documents: Option<bool>,
    pub max_chunks_per_doc: Option<i64>,
    pub kwargs: IndexMap<String, Json>,
}

// What one call of the sync callee hands back: a finished record or a coroutine to await exactly once
pub enum RerankWork<'call> {
    Immediate(Box<RerankResponse>),
    Deferred(RerankPending<'call>),
}

// The coroutine a call leaves pending; Send because the facade awaits it on the caller's task
pub type RerankPending<'call> =
    Pin<Box<dyn Future<Output = Result<RerankResponse, RerankException>> + Send + 'call>>;

// The callee the executor runs: it finishes inside begin or leaves a coroutine, the way run_in_executor does
pub trait RerankEntry {
    fn begin<'call>(
        &'call self,
        call: RerankCall,
        logging: &'call mut LoggingState,
    ) -> Result<RerankWork<'call>, RerankException>;
}

// The preliminary identity lookup and the callee, borrowable for as long as the facade runs
pub struct AsyncRerankPlan<'plan, Identity, Entry> {
    pub identity: &'plan Identity,
    pub entry: &'plan Entry,
    pub redactor: &'plan Redactor,
}

// The production entry: B1079's dispatcher always hands back its coroutine
pub struct PlanEntry<'plan, Identity, Operations> {
    pub plan: &'plan RerankPlan<'plan, Identity, Operations>,
}

impl<Identity, Operations> RerankEntry for PlanEntry<'_, Identity, Operations>
where
    Identity: RerankIdentity + Sync,
    Operations: RerankOperations + Sync,
{
    fn begin<'call>(
        &'call self,
        call: RerankCall,
        logging: &'call mut LoggingState,
    ) -> Result<RerankWork<'call>, RerankException> {
        Ok(RerankWork::Deferred(Box::pin(rerank(
            self.plan, call, logging,
        ))))
    }
}

// LiteLLM's @client arerank: resolve the identity for attribution, run the sync callee, await its coroutine once
pub async fn arerank<Identity, Entry>(
    plan: &AsyncRerankPlan<'_, Identity, Entry>,
    args: AsyncRerankArgs,
    logging: &mut LoggingState,
) -> Result<RerankResponse, RerankException>
where
    Identity: RerankIdentity + Sync,
    Entry: RerankEntry + Sync,
{
    let provider =
        attribution(plan.identity, &args).map_err(|failure| raised(plan.redactor, failure))?;
    let call = into_call(args, &provider).map_err(|failure| raised(plan.redactor, failure))?;
    match plan.entry.begin(call, logging) {
        Ok(RerankWork::Immediate(response)) => Ok(*response),
        Ok(RerankWork::Deferred(pending)) => pending
            .await
            .map_err(|failure| remapped(plan.redactor, failure)),
        Err(failure) => Err(remapped(plan.redactor, failure)),
    }
}

// declared_authenticating_provider runs first; get_llm_provider only ever feeds the error attribution
fn attribution<Identity>(
    identity: &Identity,
    args: &AsyncRerankArgs,
) -> Result<String, Box<RawException>>
where
    Identity: RerankIdentity,
{
    if let Some(provider) =
        authenticating_provider(Some(&args.model), args.custom_llm_provider.as_deref())
    {
        return Ok(String::from(<&'static str>::from(provider)));
    }
    let api_base = api_base_argument(args)?;
    identity
        .resolve(&ResolutionRequest {
            model: &args.model,
            custom_llm_provider: args.custom_llm_provider.as_deref(),
            api_base,
            api_key: None,
        })
        .map(|resolved| resolved.provider.slug().to_owned())
        // get_llm_provider raises BadRequestError with llm_provider "", so exception_type hands it back unchanged
        .map_err(|error| {
            Box::new(provider_failure(
                400,
                &error.to_string(),
                &args.model,
                "",
                "BadRequestError",
            ))
        })
}

// get_llm_provider reads kwargs["api_base"]; a value that is not text names the field, as the call itself would
fn api_base_argument(args: &AsyncRerankArgs) -> Result<Option<&str>, Box<RawException>> {
    match args.kwargs.get("api_base") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(base)) => Ok(Some(base)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(Box::new(provider_failure(
                400,
                "api_base must be a string",
                &args.model,
                "",
                "BadRequestError",
            )))
        }
    }
}

// arerank passes return_documents=None, so None stays None instead of taking rerank's True default
fn into_call(mut args: AsyncRerankArgs, provider: &str) -> Result<RerankCall, Box<RawException>> {
    let max_tokens_per_doc = max_tokens_argument(&mut args.kwargs, &args.model, provider)?;
    args.kwargs
        .insert(String::from("arerank"), Json::Bool(true));
    Ok(RerankCall {
        model: args.model,
        query: args.query,
        documents: args.documents,
        custom_llm_provider: args.custom_llm_provider,
        top_n: args.top_n,
        rank_fields: args.rank_fields,
        return_documents: args
            .return_documents
            .map_or(ReturnDocuments::Null, ReturnDocuments::Value),
        max_chunks_per_doc: args.max_chunks_per_doc,
        max_tokens_per_doc,
        kwargs: args.kwargs,
    })
}

// arerank declares no max_tokens_per_doc, so a kwargs entry binds to rerank's named parameter and leaves kwargs
fn max_tokens_argument(
    kwargs: &mut IndexMap<String, Json>,
    model: &str,
    provider: &str,
) -> Result<Option<i64>, Box<RawException>> {
    let field = "max_tokens_per_doc must be an integer";
    match kwargs.shift_remove("max_tokens_per_doc") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => number
            .as_i64()
            .map(Some)
            .ok_or_else(|| Box::new(value_failure(field, model, provider))),
        Some(Json::Text(_) | Json::Bool(_) | Json::List(_) | Json::Object(_)) => {
            Err(Box::new(value_failure(field, model, provider)))
        }
    }
}

// The facade's own failure, which exception_type maps by status before any provider arm
fn raised(redactor: &Redactor, failure: Box<RawException>) -> RerankException {
    let message = redactor.redact_string(&failure.error_str);
    RerankException {
        class: dispatch(&DispatchInput::Raw(failure), redactor),
        message,
    }
}

// exception_type returns a LiteLLM exception it already classified unchanged, so the second pass is that identity
fn remapped(redactor: &Redactor, failure: RerankException) -> RerankException {
    RerankException {
        class: dispatch(&DispatchInput::Mapped(failure.class), redactor),
        message: failure.message,
    }
}
