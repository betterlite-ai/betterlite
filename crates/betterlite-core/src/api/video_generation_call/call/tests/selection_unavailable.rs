use betterlite_error::error::ErrorClass;

use super::support::{Bench, Outcome, failed, handles, lookups, request};

#[test]
fn a_provider_whose_config_is_not_built_keeps_the_selection_message() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.custom_llm_provider = Some("edenai".to_owned());
    let raised = failed(bench.run(call));
    assert_eq!(
        raised.error.to_string(),
        "no video config is installed for 'edenai'"
    );
    let ErrorClass::BadRequest(details) = &raised.class else {
        panic!("a selection failure answers 400, got {:?}", raised.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.message, "no video config is installed for 'edenai'");
    assert_eq!(details.model.as_deref(), Some("sora-2"));
    assert_eq!(details.provider.as_deref(), Some("edenai"));
    assert!(lookups(&bench.journal).is_empty());
    assert!(handles(&bench.journal).is_empty());
}
