use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::payloads::output_choices::{finish_reasons, output_choices};
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    OutputChoices { response: MetadataMap },
    FinishReasons { choices: Vec<MetadataMap> },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match input {
            Input::OutputChoices { response } => json!(output_choices(response)),
            Input::FinishReasons { choices } => json!(finish_reasons(choices)),
        };
        Ok(json!({"result":result}))
    }
}
