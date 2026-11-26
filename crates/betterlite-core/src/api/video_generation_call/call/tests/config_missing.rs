use betterlite_error::error::ErrorClass;

use super::support::{Bench, Outcome, failed, handles, lookups, request};
use crate::api::videos_http_transport::video_config::Params;

#[test]
fn a_selector_answer_without_an_installed_config_is_refused() {
    let mut bench = Bench::with(Params::new(), true, Outcome::Answers);
    let raised = failed(bench.run(request()));
    assert_eq!(
        raised.error.to_string(),
        "no video config is installed for 'openai'"
    );
    let ErrorClass::BadRequest(details) = &raised.class else {
        panic!("a missing config answers 400, got {:?}", raised.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.message, "no video config is installed for 'openai'");
    assert_eq!(details.model.as_deref(), Some("sora-2"));
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(
        lookups(&bench.journal),
        vec!["OpenAIVideoConfig".to_owned()]
    );
    assert!(handles(&bench.journal).is_empty());
    assert!(bench.caller.providers.config.calls().is_empty());
}
