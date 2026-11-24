#![cfg(test)]

mod errors;
mod handoff;
mod identity;
mod prompt;
mod support;

#[tokio::test]
async fn async_create_parameter_handoff() {
    handoff::parameter_handoff().await;
}

#[tokio::test]
async fn asynchronous_prompt_runs_once() {
    prompt::async_prompt_hooks().await;
    prompt::prompt_outcomes().await;
}

#[tokio::test]
async fn async_create_result_identity() {
    identity::result_identity().await;
}

#[tokio::test]
async fn async_create_failure_attribution() {
    errors::failure_attribution().await;
}
