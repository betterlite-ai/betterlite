use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperResult};

mod outcome;
mod payload;
mod poster;

const FIXED_NOW: f64 = 1_000_000.0;
const UUID_MARKER: &str = "<uuid4>";

type Data = BTreeMap<String, GuardrailValue>;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptedReply {
    status: u16,
    body: Value,
    raw: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Token {
        assertion: String,
    },
    TokenRange {
        prefix: String,
        count: usize,
    },
    Post {
        url: String,
        headers: BTreeMap<String, String>,
        data: BTreeMap<String, String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TokenExchangeArgs {
    replies: Vec<ScriptedReply>,
    repeat_last_reply: bool,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssertionArgs {
    values: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PayloadAssemblyArgs {
    key_alias: Option<String>,
    mcp_session_id: Option<String>,
    litellm_call_id: String,
    metadata: Option<Data>,
    litellm_metadata: Option<Data>,
    mcp_tool_name: Option<String>,
    mcp_server_name: Option<String>,
    mcp_arguments: Option<Data>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Agent365TokenExchange(Box<TokenExchangeArgs>),
    Agent365Assertion(AssertionArgs),
    Agent365PayloadAssembly(Box<PayloadAssemblyArgs>),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Agent365TokenExchange(args) => outcome::exchange_result(args),
            Input::Agent365Assertion(args) => outcome::assertion_result(&args.values),
            Input::Agent365PayloadAssembly(args) => payload::payload_result(args),
        }
    }
}
