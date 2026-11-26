use betterlite_error::error::ErrorClass;

use super::support::{Bench, Outcome, failed, handles, lookups, request};

#[test]
fn a_provider_without_a_video_config_is_refused_before_the_options() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.custom_llm_provider = Some("anthropic".to_owned());
    let raised = failed(bench.run(call));
    assert_eq!(
        raised.error.to_string(),
        "video generation is not supported for anthropic"
    );
    let ErrorClass::BadRequest(details) = &raised.class else {
        panic!(
            "an unsupported provider answers 400, got {:?}",
            raised.class
        );
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(
        details.message,
        "video generation is not supported for anthropic"
    );
    assert_eq!(details.model.as_deref(), Some("sora-2"));
    assert_eq!(details.provider.as_deref(), Some("anthropic"));
    assert!(lookups(&bench.journal).is_empty());
    assert!(handles(&bench.journal).is_empty());
    assert!(bench.caller.providers.config.calls().is_empty());
    assert!(bench.fixture.state.optional_params.is_empty());
    assert!(bench.fixture.state.call_type.is_empty());
}
