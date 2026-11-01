use betterlite_websearch::loop_eligibility::WebSearchInterceptionState;
use betterlite_websearch::tools::Tool;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    response: Box<RawValue>,
    #[serde(default)]
    tools: Vec<Tool>,
    #[serde(default)]
    stream: bool,
    custom_llm_provider: String,
    #[serde(default)]
    surface: Option<String>,
    enabled_providers: Vec<String>,
}

#[derive(Serialize)]
struct Output {
    should_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools_dict: Option<betterlite_websearch::loop_eligibility::ToolsDict>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let state = WebSearchInterceptionState {
            enabled_providers: Some(input.enabled_providers.clone()),
        };
        let result = state
            .async_should_run_agentic_loop(
                input.response.get(),
                "test-model",
                &[],
                Some(input.tools.as_slice()),
                input.stream,
                &input.custom_llm_provider,
                input.surface.as_deref(),
            )
            .map_err(failure)?;
        let should_run = result.is_some();
        serde_json::to_value(Output {
            should_run,
            tools_dict: result,
        })
        .map_err(failure)
    }
}

fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: std::fmt::Display,
{
    let message = error.to_string();
    Box::new(crate::helpers::HelperError {
        class: "LoopEligibilityError".to_owned(),
        message,
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}
