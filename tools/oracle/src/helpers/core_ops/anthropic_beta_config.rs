use betterlite_core::api::anthropic_beta_config::matrix::{BetaNode, validate_beta_matrix};
use betterlite_oracle_helper_api::error::HelperError;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Validate { matrix: BetaNode, provider: String },
    ProviderName { matrix: BetaNode, provider: String },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Validate {
                matrix,
                provider: _provider,
            } => {
                let mut answer = Map::new();
                answer.insert(
                    "valid".to_owned(),
                    Value::Bool(validate_beta_matrix(matrix.clone()).is_ok()),
                );
                Ok(Value::Object(answer))
            }
            Input::ProviderName { matrix, provider } => {
                let validated = validate_beta_matrix(matrix.clone()).map_err(|shape| {
                    Box::new(HelperError {
                        class: "InvalidMatrix".to_owned(),
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                        message: format!("{shape:?}"),
                    })
                })?;
                Ok(Value::String(validated.provider_name(provider).to_owned()))
            }
        }
    }
}
