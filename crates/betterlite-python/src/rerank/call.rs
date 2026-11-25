use super::convert::{
    HiddenFacts, RerankAnswer, RerankCallWire, call_id, declared_api_base, hidden_params,
    model_info_id,
};
use super::error::CallFailure;
use super::operations::Operations;
use super::session::RerankSession;
use crate::completion::setup::global_credentials;
use betterlite_core::api::rerank_call::{AsyncRerankPlan, PlanEntry, arerank};
use betterlite_core::api::rerank_provider_plan::plan::{CatalogIdentity, RerankPlan, rerank};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use std::time::Instant;

// aembedding stamps the provider on every answer it awaits, and arerank measures the wrapper's overhead
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    Sync,
    Async,
}

// the identity set_hidden_params reports: the call's raw model, its declared base, its ids and its call id
struct CallFacts {
    call_id: String,
    api_base: Option<String>,
    model_id: Option<Json>,
    model: String,
}

impl CallFacts {
    #[cfg_attr(test, mutants::skip)]
    fn read(wire: &RerankCallWire) -> Self {
        Self {
            call_id: call_id(&wire.kwargs),
            api_base: declared_api_base(&wire.kwargs),
            model_id: model_info_id(&wire.kwargs).cloned(),
            model: wire.arguments.model.clone(),
        }
    }
}

// main.py's rerank call, from the resolved provider to the answer Python turns into a RerankResponse.
#[cfg_attr(test, mutants::skip)]
pub(crate) async fn call(
    session: &RerankSession,
    wire: RerankCallWire,
    entry: Entry,
) -> Result<RerankAnswer, CallFailure> {
    let started = Instant::now();
    let settings = session.settings(&wire.settings);
    let cohere = session.cohere_settings(&wire.settings);
    let host = session.host_settings();
    let globals = global_credentials(&wire.settings);
    let context = session.context(&globals);
    let identity = CatalogIdentity { context: &context };
    let facts = CallFacts::read(&wire);
    let operations = Operations::new(session, &cohere, &host);
    let mut logging =
        session.logging(call_type(entry), &facts.model, &facts.call_id, &wire.kwargs)?;
    let plan = RerankPlan {
        identity: &identity,
        operations: &operations,
        settings: &settings,
        environment: &session.environment,
        redactor: &session.redactor,
    };
    let response = dispatch(&plan, session, wire, &mut logging, entry).await?;
    Ok(answer(started, entry, &facts, &operations, response))
}

// the two call types pyc's CallTypes names for these entry points
#[cfg_attr(test, mutants::skip)]
const fn call_type(entry: Entry) -> &'static str {
    match entry {
        Entry::Sync => "rerank",
        Entry::Async => "arerank",
    }
}

// The sync entry awaits its plan directly; arerank awaits the facade's coroutine exactly once.
#[cfg_attr(test, mutants::skip)]
async fn dispatch<'plan>(
    plan: &'plan RerankPlan<'plan, CatalogIdentity<'_, '_>, Operations<'_>>,
    session: &RerankSession,
    wire: RerankCallWire,
    logging: &mut LoggingState,
    entry: Entry,
) -> Result<RerankResponse, CallFailure> {
    let response = match entry {
        Entry::Sync => rerank(plan, wire.into_call(), logging).await,
        Entry::Async => {
            let deferred = PlanEntry { plan };
            let facade = AsyncRerankPlan {
                identity: plan.identity,
                entry: &deferred,
                redactor: &session.redactor,
            };
            arerank(&facade, wire.into_async_args(), logging).await
        }
    };
    response.map_err(CallFailure::from)
}

// The hidden params set_hidden_params writes after the call, beside the response main.py validates.
#[cfg_attr(test, mutants::skip)]
fn answer(
    started: Instant,
    entry: Entry,
    facts: &CallFacts,
    operations: &Operations<'_>,
    response: RerankResponse,
) -> RerankAnswer {
    let provider = operations.provider().unwrap_or_default();
    let response_ms = started.elapsed().as_secs_f64().mul_add(1000.0_f64, 0.0_f64);
    let provider_ms = operations.provider_ms();
    let overhead = match entry {
        Entry::Sync => None,
        Entry::Async => Some(provider_ms.mul_add(-1.0_f64, response_ms).max(0.0_f64)),
    };
    let hidden = hidden_params(&HiddenFacts {
        call_id: &facts.call_id,
        api_base: facts.api_base.as_deref(),
        litellm_model: &facts.model,
        model_id: facts.model_id.as_ref(),
        provider: &provider,
        response_ms,
        overhead_ms: overhead,
    });
    RerankAnswer {
        response,
        provider,
        hidden_params: hidden,
        response_ms,
    }
}
