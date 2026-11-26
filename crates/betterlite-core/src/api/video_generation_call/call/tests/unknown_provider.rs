use betterlite_error::error::ErrorClass;

use super::support::{Bench, Outcome, failed, handles, lookups, request};

#[test]
fn a_provider_name_the_enum_does_not_know_is_refused() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.custom_llm_provider = Some("not-a-provider".to_owned());
    let raised = failed(bench.run(call));
    assert_eq!(
        raised.error.to_string(),
        "'not-a-provider' is not a valid LlmProviders"
    );
    let ErrorClass::BadRequest(details) = &raised.class else {
        panic!("an unknown provider answers 400, got {:?}", raised.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(
        details.message,
        "'not-a-provider' is not a valid LlmProviders"
    );
    assert_eq!(details.model.as_deref(), Some("not-a-provider/sora-2"));
    assert_eq!(details.provider.as_deref(), Some("not-a-provider"));
    assert!(lookups(&bench.journal).is_empty());
    assert!(handles(&bench.journal).is_empty());
}
