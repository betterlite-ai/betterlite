use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;

use crate::custom_code_execution::sandbox::CompiledProgram;
use crate::custom_code_execution::sandbox::DEFAULT_FILENAME;
use crate::custom_code_execution::sandbox::compile_sandboxed;
use crate::generic_guardrail_contracts::GuardrailInputType;

use config::CustomCodeGuardrailParams;
use config::DEFAULT_GUARDRAIL_NAME;
use config::resolve_timeout;
use error::CustomCodeApplyError;
use error::CustomCodeGuardrailError;
use events::CustomCodeEventHook;
use inputs::CustomCodeInputs;
use prepare::SafeRequestData;
use runtime::CallArguments;
use runtime::LoadedModule;
use runtime::ModuleHost;

pub mod config;
mod dispatch;
pub mod error;
pub mod events;
pub mod inputs;
mod prepare;
mod records;
mod result;
pub mod runtime;

mod tests;

const APPLY_GUARDRAIL: &str = "apply_guardrail";

// ast-grep-ignore: no-long-names
pub const RESPONSE_REJECTION_GUARDRAIL_CODE: &str = r#"
def apply_guardrail(inputs, request_data, input_type):
    """Block responses that indicate the model rejected the user request."""
    if input_type != "response":
        return allow()

    texts = inputs.get("texts") or []
    # All lowercase for case-insensitive matching (text is lowercased before check)
    rejection_phrases = [
        "that's not something i can help with",
        "that is not something i can help with",
        "i can't help with that",
        "i cannot help with that",
        "i'm not able to help",
        "i am not able to help",
        "i'm unable to help",
        "i cannot assist",
        "i can't assist",
        "i'm not allowed to",
        "i'm not permitted to",
        "i won't be able to help",
        "i'm sorry, i can't",
        "i'm sorry, i cannot",
        "as an ai, i can't",
        "as an ai, i cannot",
    ]

    for text in texts:
        if not text:
            continue
        text_lower = lower(text)
        for phrase in rejection_phrases:
            if contains(text_lower, phrase):
                return block(
                    "Response indicates the model rejected the user request.",
                    detection_info={"matched_phrase": phrase, "input_type": "response"},
                )
    return allow()
"#;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CustomCodeGuardrailConfigModel {
    pub custom_code: String,
    #[serde(default)]
    pub timeout: Option<f64>,
}

pub struct CustomCodeGuardrail {
    pub custom_code: String,
    pub guardrail_name: String,
    pub execution_timeout: f64,
    pub host: Arc<dyn ModuleHost>,
    pub clock: fn() -> Duration,
    pub execution_budget: Duration,
    // LiteLLM's _compile_lock lets only one caller compile, and every later caller reuses the module.
    pub compile_lock: Mutex<CompileState>,
}

#[derive(Default)]
pub struct CompileState {
    pub code: Option<Arc<CompiledCode>>,
    pub error: Option<String>,
}

pub struct CompiledCode {
    pub program: CompiledProgram,
    pub module: Arc<dyn LoadedModule>,
}

pub trait CallbackRegistry {
    fn add_litellm_callback(&mut self, guardrail: Arc<CustomCodeGuardrail>);
}

impl CustomCodeGuardrail {
    pub fn new(
        params: CustomCodeGuardrailParams,
        host: Arc<dyn ModuleHost>,
        clock: fn() -> Duration,
    ) -> Result<Self, CustomCodeGuardrailError> {
        let (execution_timeout, execution_budget) = resolve_timeout(params.timeout)?;
        let guardrail = Self {
            custom_code: params
                .custom_code
                .ok_or(CustomCodeGuardrailError::MissingCustomCode)?,
            guardrail_name: params
                .guardrail_name
                .unwrap_or_else(|| DEFAULT_GUARDRAIL_NAME.to_owned()),
            execution_timeout,
            execution_budget,
            host,
            clock,
            compile_lock: Mutex::new(CompileState::default()),
        };
        guardrail.compile_custom_code()?;
        Ok(guardrail)
    }

    #[must_use]
    pub const fn get_supported_event_hooks() -> [CustomCodeEventHook; 6] {
        [
            CustomCodeEventHook::PreCall,
            CustomCodeEventHook::DuringCall,
            CustomCodeEventHook::PostCall,
            CustomCodeEventHook::PreMcpCall,
            CustomCodeEventHook::DuringMcpCall,
            CustomCodeEventHook::LoggingOnly,
        ]
    }

    pub async fn apply_guardrail(
        &self,
        inputs: CustomCodeInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
    ) -> Result<CustomCodeInputs, CustomCodeApplyError> {
        let code = dispatch::compiled_code(self)?;
        let started = (self.clock)();
        let safe_request_data = Self::prepare_safe_request_data(request_data);
        let result = self
            .call_compiled(&code, &inputs, &safe_request_data, input_type)
            .await?;
        let moment = result::Moment {
            input_type,
            started,
        };
        self.process_result(result, inputs, request_data, moment)
    }

    fn compile_custom_code(&self) -> Result<Arc<CompiledCode>, CustomCodeGuardrailError> {
        let mut state = self
            .compile_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(code) = &state.code {
            return Ok(Arc::clone(code));
        }
        let program = match compile_sandboxed(&self.custom_code, DEFAULT_FILENAME) {
            Ok(program) => program,
            Err(source) => {
                let error = CustomCodeGuardrailError::Syntax { source };
                state.error = Some(error.to_string());
                return Err(error);
            }
        };
        let module = match dispatch::load_module(self, &program) {
            Ok(module) => module,
            Err(source) => {
                let error = error::load_error(source);
                state.error = Some(error.to_string());
                return Err(error);
            }
        };
        let code = Arc::new(CompiledCode {
            program,
            module: Arc::from(module),
        });
        state.code = Some(Arc::clone(&code));
        drop(state);
        tracing::debug!(
            "Custom code guardrail '{}' compiled successfully",
            self.guardrail_name
        );
        Ok(code)
    }

    // B3802 reports a run past its budget as ExecutionTimeoutError; execution_error converts it.
    async fn call_compiled(
        &self,
        code: &CompiledCode,
        inputs: &CustomCodeInputs,
        safe_request_data: &SafeRequestData,
        input_type: GuardrailInputType,
    ) -> Result<GuardrailValue, CustomCodeGuardrailError> {
        let arguments = CallArguments {
            inputs: inputs.to_value(),
            request_data: safe_request_data.to_value(),
            input_type,
        };
        let mode = if code.program.is_async(APPLY_GUARDRAIL).unwrap_or(false) {
            dispatch::CallMode::Async
        } else {
            dispatch::CallMode::Sync
        };
        dispatch::call(self, &code.module, arguments, input_type, mode).await
    }

    // LiteLLM's _prepare_safe_request_data keeps only these five keys; credentials stay out.
    fn prepare_safe_request_data(
        request_data: &BTreeMap<String, GuardrailValue>,
    ) -> SafeRequestData {
        let metadata = prepare::merged_metadata(request_data);
        SafeRequestData {
            model: request_data
                .get("model")
                .cloned()
                .unwrap_or(GuardrailValue::Null),
            user_id: metadata
                .get("user_api_key_user_id")
                .cloned()
                .unwrap_or(GuardrailValue::Null),
            team_id: metadata
                .get("user_api_key_team_id")
                .cloned()
                .unwrap_or(GuardrailValue::Null),
            end_user_id: metadata
                .get("user_api_key_end_user_id")
                .cloned()
                .unwrap_or(GuardrailValue::Null),
            metadata,
        }
    }

    // LiteLLM's _process_result checks the untrusted result object before it reaches the inputs.
    fn process_result(
        &self,
        result: GuardrailValue,
        inputs: CustomCodeInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        moment: result::Moment,
    ) -> Result<CustomCodeInputs, CustomCodeApplyError> {
        let result_type = result::value_type_name(&result);
        let GuardrailValue::Object(fields) = result else {
            tracing::warn!(
                "Custom code guardrail '{}': Expected dict result, got {result_type}. Treating as allow.",
                self.guardrail_name
            );
            return Ok(inputs);
        };
        result::dispatch(self, &fields, inputs, request_data, moment)
    }
}

pub fn initialize_guardrail<R>(
    params: CustomCodeGuardrailParams,
    host: Arc<dyn ModuleHost>,
    clock: fn() -> Duration,
    registry: &mut R,
) -> Result<Arc<CustomCodeGuardrail>, CustomCodeGuardrailError>
where
    R: CallbackRegistry,
{
    if params.guardrail_name.is_none() {
        return Err(CustomCodeGuardrailError::MissingGuardrailName);
    }
    if params.custom_code.is_none() {
        return Err(CustomCodeGuardrailError::MissingCustomCode);
    }
    let guardrail = Arc::new(CustomCodeGuardrail::new(params, host, clock)?);
    registry.add_litellm_callback(Arc::clone(&guardrail));
    Ok(guardrail)
}
