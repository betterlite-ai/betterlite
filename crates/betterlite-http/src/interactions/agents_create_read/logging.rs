use secrecy::SecretString;

use crate::interactions::base::{Params, RequestHeaders};

// the arguments LiteLLM's pre_call receives, with the complete_input_dict of create and None for a GET
pub struct AgentPreCall<'record> {
    pub input: &'record str,
    pub api_key: &'record SecretString,
    pub api_base: &'record str,
    pub headers: &'record RequestHeaders,
    pub complete_input_dict: Option<&'record Params>,
}

// the arguments LiteLLM's post_call receives; list and get pass an empty additional_args
pub struct AgentPostCall<'record> {
    pub original_response: &'record str,
    pub complete_input_dict: Option<&'record Params>,
}

pub trait AgentLogging: Send {
    fn pre_call(&mut self, call: AgentPreCall<'_>);
    fn post_call(&mut self, call: AgentPostCall<'_>);
}
