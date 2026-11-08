mod attachment;
mod matcher;
mod registry;
mod resolver;
mod support;
mod tests;

use serde::Deserialize;

use crate::helpers::HelperResult;

#[derive(Deserialize)]
pub(crate) enum Input {
    #[serde(rename = "policy_memory_matching_registry")]
    Registry(registry::Action),
    #[serde(rename = "policy_memory_matching_attachment")]
    Attachment(attachment::Action),
    #[serde(rename = "policy_memory_matching_matcher")]
    Matcher(matcher::Action),
    #[serde(rename = "policy_memory_matching_resolver")]
    Resolver(resolver::Action),
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Registry(action) => registry::run(action),
            Input::Attachment(action) => attachment::run(action),
            Input::Matcher(action) => matcher::run(action),
            Input::Resolver(action) => resolver::run(action),
        }
    }
}
