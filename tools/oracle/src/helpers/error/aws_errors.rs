use betterlite_error::aws_errors::bedrock::{self, BedrockExceptionInput};
use betterlite_error::aws_errors::sagemaker::{self, SagemakerExceptionInput};

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Bedrock(BedrockExceptionInput),
    Sagemaker(SagemakerExceptionInput),
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mapped = match input {
            Input::Bedrock(bedrock_input) => bedrock::map_bedrock_exception(bedrock_input),
            Input::Sagemaker(sagemaker_input) => {
                sagemaker::map_sagemaker_exception(sagemaker_input)
            }
        };
        Ok(serde_json::json!({ "result": mapped }))
    }
}
