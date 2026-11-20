use std::str::FromStr as _;
use std::sync::Arc;

use betterlite_core::schema::SchemaLimits;
use betterlite_logging::logging_state::correlation::CorrelationSettings;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_setup::RequestMap;
use betterlite_sdk::call_setup::callbacks::CallbackLists;
use betterlite_sdk::call_setup::correlation::CallOutcome;
use betterlite_sdk::call_setup::providers::ProviderRegistry;
use betterlite_sdk::call_wrapper::budget::BudgetState;
use betterlite_sdk::call_wrapper::cache::CacheHandler;
use betterlite_sdk::call_wrapper::error::{CallError, class_name};
use betterlite_sdk::call_wrapper::retry::WrapperRetryDefaults;
use betterlite_sdk::call_wrapper::state::WrapperOptions;
use betterlite_sdk::call_wrapper::{CallWrapper, SetupState, WrapperFlags};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::{Number, Value, json};
use tokio::runtime::{Builder, Runtime};

use super::input::{BudgetSpec, Input, Mode};
use super::ports::{
    DropScheduler, FixedClock, FixedIds, NoAssembly, NoCredentials, NoKnown, NoLogging,
    NoMaxTokens, NoMetadata, NoResolver, NoRules, ScriptedCache,
};
use super::target::{Target, value_of};

pub(super) fn run(input: &Input) -> HelperResult {
    let request = request_map(&input.request)?;
    let target = Target::new(input);
    let mut wrapper = wrapper(input)?;
    let runtime = runtime()?;
    let result = match input.mode {
        Mode::Sync => wrapper.call_blocking(&target, request, WrapperOptions::default(), &runtime),
        Mode::Async => runtime.block_on(wrapper.call(&target, request, WrapperOptions::default())),
    };
    Ok(answer(result, &target))
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error("Runtime", error.to_string()))
}

fn wrapper(input: &Input) -> Result<CallWrapper<NoRules>, Box<HelperError>> {
    let cache: Option<Arc<dyn CacheHandler>> =
        input.cache.as_ref().map(|spec| -> Arc<dyn CacheHandler> {
            Arc::new(ScriptedCache {
                hit: spec.hit.clone(),
            })
        });
    Ok(CallWrapper {
        rules: NoRules,
        setup: setup_state()?,
        cache,
        budget: budget_state(&input.budget)?,
        retry: WrapperRetryDefaults {
            num_retries: input.retry.num_retries,
            num_retries_per_request: input.retry.num_retries_per_request,
        },
        clock: Arc::new(FixedClock),
        ids: Arc::new(FixedIds),
        max_tokens: Arc::new(NoMaxTokens),
        stream_assembly: Arc::new(NoAssembly),
        metadata: Arc::new(NoMetadata),
        credentials: Arc::new(NoCredentials),
        logging: Arc::new(NoLogging),
        scheduler: Arc::new(DropScheduler),
        debugging: false,
        flags: flags(),
    })
}

// The litellm module state function_setup reads: no callbacks, no providers, no known logger names
fn setup_state() -> Result<SetupState, Box<HelperError>> {
    Ok(SetupState {
        lists: CallbackLists {
            input: Vec::new(),
            success: Vec::new(),
            async_success: Vec::new(),
            failure: Vec::new(),
            async_failure: Vec::new(),
        },
        providers: ProviderRegistry {
            custom_provider_map: Vec::new(),
            providers: Vec::new(),
            custom_providers: Vec::new(),
        },
        callbacks: Vec::new(),
        known: Arc::new(NoKnown),
        resolver: Arc::new(NoResolver),
        correlation_settings: CorrelationSettings::default(),
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|error| helper_error("Redaction", error.to_string()))?,
        litellm_params: IndexMap::new(),
        turn_off_message_logging: false,
        breadcrumbs: None,
    })
}

fn budget_state(spec: &BudgetSpec) -> Result<BudgetState, Box<HelperError>> {
    let current_cost = optional_decimal(spec.current_cost.as_ref())?.unwrap_or(Decimal::ZERO);
    Ok(BudgetState {
        max_budget: optional_decimal(spec.max_budget.as_ref())?,
        current_cost,
    })
}

// litellm.modify_params and litellm.enable_json_schema_validation are off in every case
fn flags() -> WrapperFlags {
    WrapperFlags {
        modify_params: false,
        enable_json_schema_validation: false,
        schema_limits: SchemaLimits::default(),
    }
}

fn request_map(value: &Json) -> Result<RequestMap, Box<HelperError>> {
    match value {
        Json::Null => Ok(IndexMap::new()),
        Json::Object(fields) => Ok(fields.clone()),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Err(helper_error(
            "InvalidInput",
            "the request must be an object".to_owned(),
        )),
    }
}

fn optional_decimal(value: Option<&Number>) -> Result<Option<Decimal>, Box<HelperError>> {
    value
        .map(|number| {
            Decimal::from_str(&number.to_string())
                .map_err(|error| helper_error("InvalidInput", error.to_string()))
        })
        .transpose()
}

fn answer(result: Result<CallOutcome, CallError>, target: &Target) -> Value {
    let (value, error) = match result {
        Ok(outcome) => (outcome_value(outcome), Value::Null),
        Err(error) => (
            Value::Null,
            json!({"error_class": class_name(&error.class)}),
        ),
    };
    let (calls, seen) = target.snapshot();
    json!({
        "result": value,
        "error": error,
        "calls": calls,
        "seen": seen,
    })
}

fn outcome_value(outcome: CallOutcome) -> Value {
    match outcome {
        CallOutcome::Response(response) | CallOutcome::ResponsesStream(response) => {
            value_of(&response)
        }
        CallOutcome::ChatStream(stream) => value_of(&stream.model_response),
    }
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
