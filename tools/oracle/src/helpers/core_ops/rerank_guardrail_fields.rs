use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_translation::cohere_rerank_guardrail::CohereRerankHandler;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Input {
        data: BTreeMap<String, GuardrailValue>,
        texts: Option<Vec<String>>,
    },
    Output {
        output: GuardrailValue,
    },
}

struct ScriptedGuardrail {
    texts: Option<Vec<String>>,
    calls: Mutex<Vec<Value>>,
}

impl GuardrailContract for ScriptedGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        request_data.insert("guardrail_seen".to_owned(), GuardrailValue::Boolean(true));
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "texts": inputs.texts,
                "model": inputs.model,
                "input_type": input_type,
                "request_data": request_data,
            }));
        ready(Ok(GuardrailApiInputs {
            texts: self.texts.clone().or(inputs.texts),
            ..inputs
        }))
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        runtime.block_on(async {
            let mut answers = Vec::with_capacity(input.calls.len());
            for call in &input.calls {
                answers.push(dispatch(call).await?);
            }
            Ok(json!(answers))
        })
    }
}

async fn dispatch(call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::Input { data, texts } => {
            let guardrail = ScriptedGuardrail {
                texts: texts.clone(),
                calls: Mutex::default(),
            };
            let mut request_data = data.clone();
            CohereRerankHandler
                .process_input_messages(&mut request_data, &guardrail, None)
                .await
                .or_value_error()?;
            let calls = guardrail
                .calls
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner);
            Ok(json!({"data": request_data, "calls": calls}))
        }
        Call::Output { output } => {
            let guardrail = ScriptedGuardrail {
                texts: None,
                calls: Mutex::default(),
            };
            let returned = CohereRerankHandler.process_output_response(output.clone(), &guardrail);
            let calls = guardrail
                .calls
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner);
            Ok(json!({"same": returned == *output, "calls": calls}))
        }
    }
}
