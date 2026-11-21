use std::collections::BTreeMap;

use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::message_content::Message;
use serde::Deserialize;

mod call;
mod hooks;
mod source;
mod tests;
mod view;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Completion,
    OSeries,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    #[serde(default)]
    cases: Vec<CallCase>,
}

// A bool on the wire; the newtype keeps struct_excessive_bools off the case input.
#[derive(Clone, Copy, Default, Deserialize)]
#[serde(transparent)]
pub(super) struct Flag(bool);

// AzureChatCompletion.completion's arguments as the oracle cases spell them
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallCase {
    pub(super) model: Option<String>,
    pub(super) messages: Option<Vec<Message>>,
    pub(super) api_key: Option<String>,
    pub(super) client_api_key: Option<String>,
    pub(super) azure_ad_token: Option<String>,
    #[serde(default = "default_api_base")]
    pub(super) api_base: String,
    #[serde(default = "default_api_version")]
    pub(super) api_version: String,
    #[serde(default = "default_timeout")]
    pub(super) timeout: f64,
    #[serde(default)]
    pub(super) optional_params: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub(super) litellm_params: LiteLlmParams,
    #[serde(default)]
    pub(super) base_model: Option<String>,
    #[serde(default)]
    pub(super) organization: Option<String>,
    #[serde(default)]
    pub(super) drop_params: Option<bool>,
    #[serde(default)]
    pub(super) acompletion: bool,
    // the SDK client the fixture builds: sync or async, azure or openai spelling
    #[serde(default)]
    pub(super) client: Option<String>,
    // false hands completion no client, which only the Cloudflare gateway can replace
    #[serde(default = "default_supplied")]
    pub(super) supplied: bool,
    #[serde(default)]
    pub(super) headers: Option<BTreeMap<String, String>>,
    #[serde(default = "default_retries")]
    pub(super) default_max_retries: i64,
    #[serde(default)]
    pub(super) openai_system_messages_first: Flag,
    #[serde(default)]
    pub(super) cancel: Flag,
    #[serde(default)]
    pub(super) answer: source::answer::Answer,
}

fn default_api_base() -> String {
    "https://unit.test".to_owned()
}

fn default_api_version() -> String {
    "2024-02-01".to_owned()
}

const fn default_timeout() -> f64 {
    30.0
}

const fn default_retries() -> i64 {
    2
}

const fn default_supplied() -> bool {
    true
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
            Function::OSeries => {
                let calls: Vec<_> = input
                    .cases
                    .iter()
                    .map(call::os_series::run)
                    .collect::<Result<_, _>>()?;
                view::calls(calls)
            }
        };
        serde_json::to_value(value).or_value_error()
    }
}
