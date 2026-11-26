use betterlite_error::error::ErrorClass;

use super::support::{Bench, Outcome, failed, request};

#[test]
fn a_handler_that_never_answers_suspends_the_blocking_call() {
    let mut bench = Bench::new(Outcome::NeverAnswers);
    let raised = failed(bench.run(request()));
    assert_eq!(
        raised.error.to_string(),
        "the blocking video generation call waited on a handler that never answered"
    );
    let ErrorClass::ApiConnection(context) = &raised.class else {
        panic!("a suspended call is a value error, got {:?}", raised.class);
    };
    assert_eq!(
        context.message,
        "APIConnectionError: OpenAIException - the blocking video generation call waited on a handler that never answered"
    );
    assert_eq!(context.model.as_deref(), Some("sora-2"));
    assert_eq!(context.provider.as_deref(), Some("openai"));
}
