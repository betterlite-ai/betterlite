use std::fmt::Debug;
use std::mem;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::api::responses_file_search_tools::outputs::{
    build_file_search_call_output, build_message_output,
};
use betterlite_core::api::responses_file_search_tools::preparation::{
    extract_tool_call_fields, prepare_file_search_call, resolve_queries_from_args,
};
use betterlite_core::api::responses_file_search_tools::results::format_results_as_tool_output;
use betterlite_core::api::responses_file_search_tools::synthesis::{
    OriginalModel, OriginalResponse, OutputItems, synthesize_response,
};
use betterlite_core::api::responses_file_search_tools::tools::{
    build_function_tool, replace_file_search_tools,
};
use betterlite_core::api::responses_tool_namespaces::values::Fields;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResult;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry;

use self::cases::{
    ArgumentPreparationCase, Input, OutputItemsCase, ResponseSynthesisCase, ResultRenderingCase,
    ToolReplacementCase,
};

mod cases;

type Failure = Box<HelperError>;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ToolSchema(case) => Ok(serde_json::to_value(build_function_tool(
                &case.vector_store_ids,
            ))?),
            Input::ToolReplacement(case) => replacement_answer(case),
            Input::ResultRendering(case) => rendering_answer(case),
            Input::OutputItems(case) => items_answer(case),
            Input::ResponseSynthesis(case) => synthesis_answer(case),
            Input::ArgumentPreparation(case) => preparation_answer(case),
        }
    }
}

fn failure(message: String) -> Failure {
    Box::new(HelperError {
        class: "FileSearchToolsError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn replacement_answer(case: &ToolReplacementCase) -> Result<Value, Failure> {
    let (kept, ids) = replace_file_search_tools(case.tools.as_deref())
        .map_err(|error| failure(error.to_string()))?;
    Ok(json!({"tools": kept, "vector_store_ids": ids}))
}

fn rendering_answer(case: &ResultRenderingCase) -> Result<Value, Failure> {
    let call = build_file_search_call_output("fs_rendered", &[], Some(&case.results), true)
        .map_err(|error| failure(error.to_string()))?;
    let output = format_results_as_tool_output(&case.results);
    Ok(json!({"include": serde_json::to_value(call.search_results)?, "output": output}))
}

fn build_outputs(
    call_id: &str,
    queries: &[String],
    results: Option<&[VectorStoreSearchResult]>,
    include_search_results: bool,
    text: &str,
) -> Result<OutputItems, Failure> {
    let call = build_file_search_call_output(call_id, queries, results, include_search_results)
        .map_err(|error| failure(error.to_string()))?;
    let message = build_message_output(text, results.unwrap_or_default());
    Ok(OutputItems { call, message })
}

fn items_answer(case: &OutputItemsCase) -> Result<Value, Failure> {
    let outputs = build_outputs(
        &case.call_id,
        &case.queries,
        case.results.as_deref(),
        case.include_search_results,
        &case.text,
    )?;
    Ok(json!({
        "call": serde_json::to_value(&outputs.call)?,
        "message": serde_json::to_value(&outputs.message)?,
    }))
}

// a response LiteLLM's model built always exposes _hidden_params, so a null mapping reads as the empty one
fn original_of(case: &ResponseSynthesisCase) -> OriginalResponse {
    let Some(view) = case.original.as_ref() else {
        return OriginalResponse::default();
    };
    OriginalResponse {
        id: Some(view.id.clone()),
        created_at: Some(view.created_at),
        model: view.model.as_ref().map_or(OriginalModel::Null, |text| {
            OriginalModel::Text(text.clone())
        }),
        usage: view.usage.clone(),
        hidden_params: Some(view.hidden_params.clone().unwrap_or_default()),
    }
}

fn synthesis_answer(case: &ResponseSynthesisCase) -> Result<Value, Failure> {
    let outputs = build_outputs(
        &case.call_id,
        &case.queries,
        Some(&case.results),
        case.include_search_results,
        &case.text,
    )?;
    let now = Decimal::from_str(&case.now).map_err(|error| failure(error.to_string()))?;
    let generated_id = case.uuid.clone();
    let synthesized = synthesize_response(
        &original_of(case),
        &outputs,
        case.first_hidden.as_ref(),
        || generated_id.clone(),
        || now,
    )
    .map_err(|error| failure(error.to_string()))?;
    Ok(json!({
        "response": serde_json::to_value(&synthesized.response)?,
        "hidden_params": synthesized.hidden_params,
    }))
}

#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<Value>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Lines {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        let level = match *event.metadata().level() {
            Level::ERROR => "ERROR",
            Level::WARN => "WARNING",
            Level::INFO => "INFO",
            Level::DEBUG => "DEBUG",
            Level::TRACE => "TRACE",
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"level": level, "message": message.0}));
    }
}

fn prepared_answer(kwargs: &Fields) -> Result<Value, Failure> {
    let lines = Lines::default();
    let guard = set_default(registry().with(lines.clone()));
    let prepared = prepare_file_search_call(kwargs);
    drop(guard);
    let call = prepared.map_err(|error| failure(error.to_string()))?;
    Ok(json!({
        "include_search_results": call.include_search_results,
        "updated_kwargs": call.updated_kwargs,
        "logs": mem::take(&mut *lines.0.lock().unwrap_or_else(PoisonError::into_inner)),
    }))
}

fn extracted_answer(tool_call: &Fields, fallback: &str) -> Result<Value, Failure> {
    let (call_id, arguments) = extract_tool_call_fields(tool_call, fallback)
        .map_err(|error| failure(error.to_string()))?;
    Ok(json!({"call_id": call_id, "arguments": arguments}))
}

fn query_answer(args: &Fields, input: &Json) -> Result<Value, Failure> {
    let queries =
        resolve_queries_from_args(args, input).map_err(|error| failure(error.to_string()))?;
    Ok(json!(queries))
}

fn preparation_answer(case: &ArgumentPreparationCase) -> Result<Value, Failure> {
    if case.query_args.len() != case.query_inputs.len() {
        return Err(failure(
            "query_args and query_inputs must have the same length".to_owned(),
        ));
    }
    let mut prepared = Vec::with_capacity(case.prepares.len());
    for kwargs in &case.prepares {
        prepared.push(prepared_answer(kwargs)?);
    }
    let mut extracted = Vec::with_capacity(case.tool_calls.len());
    for tool_call in &case.tool_calls {
        extracted.push(extracted_answer(tool_call, &case.fallback_call_id)?);
    }
    let mut queries = Vec::with_capacity(case.query_args.len());
    for (args, input) in case.query_args.iter().zip(&case.query_inputs) {
        queries.push(query_answer(args, input)?);
    }
    Ok(json!({
        "prepared": prepared,
        "extracted": extracted,
        "queries": queries,
    }))
}
