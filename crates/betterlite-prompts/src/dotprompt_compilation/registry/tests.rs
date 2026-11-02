#![cfg(test)]

use std::ptr;

use betterlite_models::prompt_spec::SupportedPromptIntegrations;

use super::{InitializerFn, prompt_initializer_registry};
use crate::dotprompt_compilation::init::factory::prompt_initializer;

#[test]
fn dot_prompt_maps_to_the_prompt_initializer_function() {
    let initializer = prompt_initializer_registry(&SupportedPromptIntegrations::DotPrompt)
        .expect("dotprompt is registered");
    let expected: InitializerFn = prompt_initializer;
    assert!(ptr::fn_addr_eq(initializer, expected));
}

#[test]
fn an_unregistered_integration_has_no_initializer() {
    assert!(prompt_initializer_registry(&SupportedPromptIntegrations::Langfuse).is_none());
}
