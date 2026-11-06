use std::ptr;

use betterlite_models::prompt_spec::SupportedPromptIntegrations;
use betterlite_prompts::bitbucket::initializer::{
    InitializerFn, prompt_initializer, prompt_initializer_registry,
};

#[test]
fn bitbucket_maps_to_the_prompt_initializer_function() {
    let initializer = prompt_initializer_registry(&SupportedPromptIntegrations::Bitbucket)
        .expect("bitbucket is registered");
    let expected: InitializerFn = prompt_initializer;
    assert!(ptr::fn_addr_eq(initializer, expected));
}

#[test]
fn another_integration_has_no_bitbucket_initializer() {
    assert!(prompt_initializer_registry(&SupportedPromptIntegrations::Gitlab).is_none());
}
