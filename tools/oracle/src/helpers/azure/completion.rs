use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_models::decisions::Extra;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::builtin::Message;
use serde::Deserialize;
use std::collections::BTreeMap;

mod call;
mod hooks;
mod source;
mod tests;
mod view;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Completion,
    Stream,
    Auth,
    Config,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    #[serde(default)]
    cases: Vec<CallCase>,
    #[serde(default)]
    constructor: Option<Extra>,
}

// AzureTextCompletion.completion's arguments as the oracle cases spell them
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallCase {
    pub(super) model: Option<String>,
    pub(super) messages: Option<Vec<Message>>,
    pub(super) api_key: Option<String>,
    pub(super) client_api_key: Option<String>,
    pub(super) azure_ad_token: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) api_version: Option<String>,
    pub(super) timeout: Option<f64>,
    #[serde(default)]
    pub(super) optional_params: Extra,
    #[serde(default)]
    pub(super) litellm_params: LiteLlmParams,
    #[serde(default)]
    pub(super) acompletion: bool,
    // the SDK client the fixture builds when the case asks for the other kind than acompletion selects
    #[serde(default)]
    pub(super) client: Option<String>,
    #[serde(default)]
    pub(super) headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) data: Extra,
    #[serde(default)]
    pub(super) answer: source::Answer,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let value = match input.function {
            Function::Completion => {
                let calls: Vec<_> = input
                    .cases
                    .iter()
                    .map(call::completion)
                    .collect::<Result<_, _>>()?;
                view::calls(calls)
            }
            Function::Stream => {
                let calls: Vec<_> = input
                    .cases
                    .iter()
                    .map(call::stream::direct_stream)
                    .collect::<Result<_, _>>()?;
                view::calls(calls)
            }
            Function::Auth => {
                let results: Vec<_> = input.cases.iter().map(call::auth).collect();
                view::results(results)
            }
            Function::Config => call::config(input.constructor.as_ref())?,
        };
        serde_json::to_value(value).or_value_error()
    }
}
