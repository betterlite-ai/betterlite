use betterlite_models::decisions::Json;

use super::support::{Bench, Outcome, handled, params, request, text};
use crate::api::video_generation_call::CALL_TYPE;

#[test]
fn the_update_carries_the_resolved_call_and_sets_the_call_type_after_it() {
    let mapped = params(&[("seconds", text("8"))]);
    let mut bench = Bench::with(mapped.clone(), false, Outcome::Answers);
    let mut call = request();
    call.user = Some("user-1".to_owned());
    call.kwargs
        .insert("litellm_call_id".to_owned(), text("call-9"));
    call.kwargs.insert("size".to_owned(), text("720x1280"));
    bench.run(call).expect("the call answers");
    let inner = handled(&bench.journal);
    assert_eq!(inner.logged.user.as_deref(), Some("user-1"));
    assert_eq!(inner.logged.optional_params, mapped);
    let state = &bench.fixture.state;
    assert_eq!(state.model, "sora-2");
    assert_eq!(state.user.as_deref(), Some("user-1"));
    assert_eq!(state.optional_params, mapped);
    assert_eq!(
        state.litellm_params,
        params(&[("litellm_call_id", text("call-9")), ("seconds", text("8")),])
    );
    assert_eq!(state.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        state.model_call_details.get("custom_llm_provider"),
        Some(&text("openai"))
    );
    assert_eq!(state.model_call_details.get("model"), Some(&text("sora-2")));
    assert_eq!(state.model_call_details.get("user"), Some(&text("user-1")));
    assert_eq!(state.call_type, CALL_TYPE);
    assert_eq!(
        state.model_call_details.get("call_type"),
        Some(&Json::Text(String::new())),
        "the update ran before the call type was set, as LiteLLM does"
    );
}
