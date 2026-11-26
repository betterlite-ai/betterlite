use betterlite_models::decisions::Json;

use super::support::{Bench, Outcome, VIDEO_ID, handled, lookups, params, request, text};
use crate::api::video_generation_call::CALL_TYPE;

#[test]
fn the_default_model_resolves_to_openai_and_sets_create_video() {
    let mut bench = Bench::new(Outcome::Answers);
    let object = bench.run(request()).expect("the call answers");
    assert_eq!(object.id, VIDEO_ID);
    let inner = handled(&bench.journal);
    assert_eq!(inner.model, "sora-2");
    assert_eq!(inner.custom_llm_provider, "openai");
    assert_eq!(inner.prompt, "a prompt");
    assert_eq!(
        lookups(&bench.journal),
        vec!["OpenAIVideoConfig".to_owned()]
    );
    let mapped = bench.caller.providers.config.calls();
    assert_eq!(mapped.len(), 1_usize);
    assert_eq!(mapped[0].model, "sora-2");
    assert!(mapped[0].drop_params);
    assert_eq!(bench.fixture.state.call_type, CALL_TYPE);
    assert_eq!(bench.fixture.state.model, "sora-2");
    assert_eq!(inner.logged.model, "sora-2");
    assert_eq!(inner.logged.call_type, CALL_TYPE);
    assert_eq!(inner.logged.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(inner.logged.provider_note, Some(text("openai")));
    assert_eq!(
        inner.logged.litellm_params,
        params(&[("litellm_call_id", Json::Null)])
    );
}

#[test]
fn an_empty_model_takes_the_default() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.model = Some(String::new());
    bench.run(call).expect("the call answers");
    let inner = handled(&bench.journal);
    assert_eq!(inner.model, "sora-2");
    assert_eq!(inner.custom_llm_provider, "openai");
}

#[test]
fn a_prefixed_model_resolves_through_its_provider() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.model = Some("openai/sora-2".to_owned());
    bench.run(call).expect("the call answers");
    let inner = handled(&bench.journal);
    assert_eq!(inner.model, "sora-2");
    assert_eq!(inner.custom_llm_provider, "openai");
    let mapped = bench.caller.providers.config.calls();
    assert_eq!(mapped[0].model, "sora-2");
}

#[test]
fn a_named_provider_resolves_a_model_without_an_openai_name() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.model = Some("some-video-model".to_owned());
    call.custom_llm_provider = Some("openai".to_owned());
    bench.run(call).expect("the call answers");
    let inner = handled(&bench.journal);
    assert_eq!(inner.model, "some-video-model");
    assert_eq!(inner.custom_llm_provider, "openai");
    let mapped = bench.caller.providers.config.calls();
    assert_eq!(mapped[0].model, "some-video-model");
}
