use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_base::{GuardrailBase, settings::GuardrailSettings};
use betterlite_guardrails::guardrail_records::error::GuardrailSpanError;
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailSpanSink,
};
use betterlite_guardrails::microsoft_purview::purview_dlp_hooks::{
    context::HookContext,
    error::PurviewHookError,
    purview_async_logging_hook::{AuditEngine, DeferredCall, async_logging_hook},
    purview_logging_hook::{AuditSpawner, logging_hook},
    purview_post_success_hook::async_post_call_success_hook,
    purview_pre_call_hook::async_pre_call_hook,
};
// ast-grep-ignore: no-long-names
use betterlite_guardrails::microsoft_purview::purview_dlp_hooks::purview_streaming_hook::async_post_call_streaming_iterator_hook as streaming_hook;
use betterlite_guardrails::purview_dlp_evaluation::{
    MicrosoftPurviewDLPGuardrail, PurviewDlpGuardrailParams,
};
use betterlite_guardrails::purview_graph_client::user_id_resolution::KeyIdentity;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use reqwest::Client;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::super::wire::{self, Function, Input};
use super::mock::{self, GraphMock};

type Outcome = Result<(), PurviewHookError>;

pub(crate) fn run(input: &Input) -> HelperResult {
    let mock = mock::spawn_mock(&input.replies)?;
    let guardrail = guardrail(&input.guardrail, &mock.base);
    let setup = Setup::new(mock, guardrail)?;
    let status = match input.function {
        Function::PreCall => response_status(pre_call(input, &setup)?),
        Function::PostSuccess => response_status(post_success(input, &setup)?),
        Function::Streaming => response_status(streaming(input, &setup)?),
        Function::LoggingSync => logging_sync(input, &setup)?,
        Function::LoggingDeferred => logging_deferred(input, &setup)?,
        Function::LoggingAsync => logging_async(input, &setup)?,
    };
    Ok(json!({"scans": setup.mock.scans(), "status": status}))
}

fn pre_call(input: &Input, setup: &Setup) -> Result<Outcome, Box<HelperError>> {
    blocking(input, setup, BlockingOperation::PreCall(call_type(input)?))
}

fn post_success(input: &Input, setup: &Setup) -> Result<Outcome, Box<HelperError>> {
    let result = required(input.result.as_ref(), "result")?.clone();
    blocking(input, setup, BlockingOperation::PostSuccess(result))
}

fn streaming(input: &Input, setup: &Setup) -> Result<Outcome, Box<HelperError>> {
    let chunks = required(input.chunks.as_ref(), "chunks")?.clone();
    blocking(input, setup, BlockingOperation::Streaming(chunks))
}

enum BlockingOperation {
    PreCall(Option<CallType>),
    PostSuccess(GuardrailValue),
    Streaming(Vec<GuardrailValue>),
}

// The three blocking hooks differ only in the call they make on one request's state.
fn blocking(
    input: &Input,
    setup: &Setup,
    operation: BlockingOperation,
) -> Result<Outcome, Box<HelperError>> {
    let runtime = runtime()?;
    let sink = DiscardSink;
    let mut data = input.data.clone();
    let mut context = setup.context(identity(input), &sink);
    let guardrail = &setup.guardrail;
    Ok(match operation {
        BlockingOperation::PreCall(call_type) => runtime.block_on(async_pre_call_hook(
            guardrail,
            &mut data,
            call_type,
            &mut context,
        )),
        BlockingOperation::PostSuccess(result) => runtime.block_on(async {
            async_post_call_success_hook(guardrail, &mut data, result, &mut context)
                .await
                .map(|_| ())
        }),
        BlockingOperation::Streaming(chunks) => runtime.block_on(async {
            streaming_hook(guardrail, chunks, &mut data, &mut context)
                .await
                .map(|_| ())
        }),
    })
}

fn logging_sync(input: &Input, setup: &Setup) -> Result<Option<u16>, Box<HelperError>> {
    let call = DeferredCall {
        kwargs: input.data.clone(),
        result: required(input.result.as_ref(), "result")?.clone(),
        call_type: call_type(input)?,
    };
    let _returned = logging_hook(&setup.engine, call, &InlineSpawner);
    Ok(None)
}

// Inside a runtime the sync hook defers to the framework's async hook, so nothing is scanned here.
fn logging_deferred(input: &Input, setup: &Setup) -> Result<Option<u16>, Box<HelperError>> {
    let runtime = runtime()?;
    let _entered = runtime.enter();
    logging_sync(input, setup)
}

fn logging_async(input: &Input, setup: &Setup) -> Result<Option<u16>, Box<HelperError>> {
    let result = required(input.result.as_ref(), "result")?;
    let call_type = call_type(input)?;
    let mut kwargs = input.data.clone();
    let engine = &setup.engine;
    let runtime = runtime()?;
    runtime.block_on(async_logging_hook(engine, &mut kwargs, result, call_type));
    Ok(None)
}

fn identity(input: &Input) -> KeyIdentity<'_> {
    let Some(identity) = &input.identity else {
        return KeyIdentity::default();
    };
    KeyIdentity {
        user_id: identity.user_id.as_deref(),
        end_user_id: identity.end_user_id.as_deref(),
    }
}

fn call_type(input: &Input) -> Result<Option<CallType>, Box<HelperError>> {
    let Some(name) = &input.call_type else {
        return Ok(None);
    };
    let value = Value::String(name.clone());
    serde_json::from_value::<CallType>(value)
        .map(Some)
        .or_value_error()
}

fn guardrail(config: &wire::GuardrailConfig, base: &str) -> MicrosoftPurviewDLPGuardrail {
    let params = PurviewDlpGuardrailParams {
        guardrail_name: config.guardrail_name.clone(),
        tenant_id: config.tenant_id.clone(),
        client_id: config.client_id.clone(),
        client_secret: SecretString::from(config.client_secret.clone()),
        purview_app_name: config.purview_app_name.clone(),
        user_id_field: config.user_id_field.clone(),
        timeout: config.timeout_seconds.map(Duration::from_secs),
    };
    let mut guardrail = MicrosoftPurviewDLPGuardrail::new(params, Client::default());
    guardrail.base.graph_api_base = base.to_owned();
    guardrail.base.login_authority = base.to_owned();
    guardrail
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .or_config_error()
}

fn required<'value, T>(
    value: Option<&'value T>,
    field: &str,
) -> Result<&'value T, Box<HelperError>> {
    value.ok_or_else(|| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!("{field} is required for this function"),
        })
    })
}

fn response_status(outcome: Outcome) -> Option<u16> {
    match outcome {
        Ok(()) => None,
        Err(PurviewHookError::Http(error)) => Some(error.status),
        Err(PurviewHookError::Record { .. }) => Some(500),
    }
}

struct Setup {
    mock: GraphMock,
    guardrail: Arc<MicrosoftPurviewDLPGuardrail>,
    record_base: GuardrailBase,
    cache: Arc<DualLocalCache>,
    redactor: Arc<Redactor>,
    engine: AuditEngine,
}

impl Setup {
    fn new(
        mock: GraphMock,
        guardrail: MicrosoftPurviewDLPGuardrail,
    ) -> Result<Self, Box<HelperError>> {
        let redactor = Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?);
        let diagnostics = Diagnostics {
            redactor: Arc::clone(&redactor),
            emit: Arc::new(|_event| {}),
        };
        let cache = Arc::new(DualLocalCache::new(
            None,
            None,
            &Settings::default(),
            Arc::new(|| 0.0_f64),
            diagnostics,
        ));
        let record_base =
            GuardrailBase::new(GuardrailSettings::default(), false).or_config_error()?;
        let guardrail = Arc::new(guardrail);
        let engine = AuditEngine {
            guardrail: Arc::clone(&guardrail),
            record_base: record_base.clone(),
            cache: Arc::clone(&cache),
            sink: Arc::new(DiscardSink),
        };
        Ok(Self {
            mock,
            guardrail,
            record_base,
            cache,
            redactor,
            engine,
        })
    }

    fn context<'setup>(
        &'setup self,
        identity: KeyIdentity<'setup>,
        sink: &'setup DiscardSink,
    ) -> HookContext<'setup> {
        HookContext {
            identity,
            cache: Arc::clone(&self.cache),
            redactor: Arc::clone(&self.redactor),
            record_base: &self.record_base,
            invocation: None,
            sink,
            log_target: None,
        }
    }
}

struct DiscardSink;

impl GuardrailSpanSink for DiscardSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

struct InlineSpawner;

impl AuditSpawner for InlineSpawner {
    fn spawn(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        job();
    }
}

mod tests;
