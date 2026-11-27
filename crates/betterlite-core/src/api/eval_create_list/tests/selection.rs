use betterlite_context::resource_config_selection::skills_evals::EvalsConfigTag;
use betterlite_error::error::ErrorClass;
use betterlite_openai::evals::openai::OpenAiEvalsConfig;

use crate::api::eval_create_list::call::{EvalCreation, EvalListing};
use crate::api::eval_create_list::config::{EvalsConfigLookup as _, EvalsConfigs};
use crate::api::eval_create_list::error::EvalsCreateListError;
use crate::api::eval_create_list::tests::support::{
    Calls, Harness, create_call, eval, evals_page, list_call,
};

#[tokio::test]
async fn create_eval_defaults_to_openai() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = create_call(&calls, None, None);
    let mut state = harness.logging_state("acreate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_create(Ok(eval("eval_1")));
    let creation = EvalCreation {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .create_eval(creation)
        .await
        .expect("the create succeeds");
    let creates = harness.handler.creates();
    assert_eq!(creates[0].provider, Some("openai".to_owned()));
    assert_eq!(harness.recorded().creates.len(), 1);
}

#[tokio::test]
async fn list_evals_defaults_to_openai() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = list_call(&calls);
    let mut state = harness.logging_state("alist_evals");
    let env = harness.env.pre_post();
    harness.handler.answer_list(Ok(evals_page()));
    let listing = EvalListing {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .list_evals(listing)
        .await
        .expect("the list succeeds");
    let lists = harness.handler.lists();
    assert_eq!(lists[0].provider, Some("openai".to_owned()));
    assert_eq!(harness.recorded().lists.len(), 1);
}

#[tokio::test]
async fn create_eval_rejects_a_provider_without_an_evals_config() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.provider = Some("anthropic".to_owned());
    let call = create_call(&calls, None, None);
    let mut state = harness.logging_state("acreate_eval");
    let env = harness.env.pre_post();
    let creation = EvalCreation {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .create_eval(creation)
        .await
        .expect_err("an unsupported provider fails the create");
    assert!(matches!(
        &failure.error,
        EvalsCreateListError::CreateUnsupported { .. }
    ));
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert_eq!(
        failure.error.to_string(),
        "CREATE eval is not supported for anthropic"
    );
    assert!(harness.handler.creates().is_empty());
    assert!(harness.recorded().validated.is_empty());
}

#[tokio::test]
async fn list_evals_rejects_a_provider_without_an_evals_config() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.provider = Some("anthropic".to_owned());
    let call = list_call(&calls);
    let mut state = harness.logging_state("alist_evals");
    let env = harness.env.pre_post();
    let listing = EvalListing {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .list_evals(listing)
        .await
        .expect_err("an unsupported provider fails the list");
    assert!(matches!(
        &failure.error,
        EvalsCreateListError::ListUnsupported { .. }
    ));
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert_eq!(
        failure.error.to_string(),
        "LIST evals is not supported for anthropic"
    );
    assert!(harness.handler.lists().is_empty());
}

#[tokio::test]
async fn create_eval_rejects_an_unknown_provider_name() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.provider = Some("nonsense".to_owned());
    let call = create_call(&calls, None, None);
    let mut state = harness.logging_state("acreate_eval");
    let env = harness.env.pre_post();
    let creation = EvalCreation {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .create_eval(creation)
        .await
        .expect_err("an unknown provider fails the create");
    assert!(matches!(
        &failure.error,
        EvalsCreateListError::UnknownProvider { .. }
    ));
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert_eq!(
        failure.error.to_string(),
        "\"nonsense\" is not a provider betterlite knows"
    );
    assert!(harness.handler.creates().is_empty());
}

#[tokio::test]
async fn list_evals_rejects_an_unknown_provider_name() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.provider = Some("nonsense".to_owned());
    let call = list_call(&calls);
    let mut state = harness.logging_state("alist_evals");
    let env = harness.env.pre_post();
    let listing = EvalListing {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .list_evals(listing)
        .await
        .expect_err("an unknown provider fails the list");
    assert!(matches!(
        &failure.error,
        EvalsCreateListError::UnknownProvider { .. }
    ));
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(harness.handler.lists().is_empty());
}

#[tokio::test]
async fn create_eval_does_not_fall_back_when_the_config_is_missing() {
    let harness = Harness::without_config();
    let calls = Calls::empty();
    let call = create_call(&calls, None, None);
    let mut state = harness.logging_state("acreate_eval");
    let env = harness.env.pre_post();
    let creation = EvalCreation {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .create_eval(creation)
        .await
        .expect_err("a missing config fails the create");
    assert_eq!(
        failure.error.to_string(),
        "CREATE eval is not supported for openai"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(harness.handler.creates().is_empty());
}

#[tokio::test]
async fn list_evals_does_not_fall_back_when_the_config_is_missing() {
    let harness = Harness::without_config();
    let calls = Calls::empty();
    let call = list_call(&calls);
    let mut state = harness.logging_state("alist_evals");
    let env = harness.env.pre_post();
    let listing = EvalListing {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .list_evals(listing)
        .await
        .expect_err("a missing config fails the list");
    assert_eq!(
        failure.error.to_string(),
        "LIST evals is not supported for openai"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(harness.handler.lists().is_empty());
}

#[test]
fn evals_configs_resolves_the_openai_tag() {
    let configs = EvalsConfigs {
        openai: OpenAiEvalsConfig::default(),
    };
    let config = configs
        .evals_config(EvalsConfigTag::OpenAI)
        .expect("the openai tag resolves");
    assert_eq!(
        config
            .get_complete_url(None, "evals", None)
            .expect("the openai config builds its url"),
        "https://api.openai.com/v1/evals"
    );
}
