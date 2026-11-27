use std::mem::size_of;

use crate::api::eval_create_list::handler::{BASE_LLM_HTTP_HANDLER, BaseLlmHttpHandler};
use crate::api::eval_create_list::{DEFAULT_EVALS_PROVIDER, DEFAULT_OPENAI_API_BASE};
use crate::api::eval_get_update::handler::EvalsGetUpdateHandler;

fn assert_handler<H>(_handler: &H)
where
    H: EvalsGetUpdateHandler + 'static,
{
}

#[test]
fn module_reuses_the_litellm_base_and_provider_defaults() {
    assert_eq!(DEFAULT_OPENAI_API_BASE, "https://api.openai.com");
    assert_eq!(DEFAULT_EVALS_PROVIDER, "openai");
}

#[test]
fn base_llm_http_handler_serves_the_get_and_update_calls() {
    assert_handler(&BASE_LLM_HTTP_HANDLER);
    assert_eq!(size_of::<BaseLlmHttpHandler>(), 0);
}
