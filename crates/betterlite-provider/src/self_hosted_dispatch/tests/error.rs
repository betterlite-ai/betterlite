use crate::petals::error::{ENGINE_MISSING_MESSAGE, ENGINE_STATUS, PetalsSetupError};
use crate::self_hosted_dispatch::error::{DispatchError, LemonadeError};
use crate::self_hosted_dispatch::handler::HandlerError;
use crate::vllm::completion::ENGINE_ERROR;

#[test]
fn dispatch_errors_report_their_status() {
    assert_eq!(
        DispatchError::Vllm(Box::new(ENGINE_ERROR)).status(),
        Some(400)
    );
    assert_eq!(
        DispatchError::Petals(PetalsSetupError::Engine {
            status_code: ENGINE_STATUS,
            message: ENGINE_MISSING_MESSAGE,
        })
        .status(),
        Some(400)
    );
    assert_eq!(
        DispatchError::Handler(HandlerError::Upstream { status_code: 429 }).status(),
        Some(429)
    );
    assert_eq!(
        DispatchError::Handler(HandlerError::EmptyReply).status(),
        None
    );
    assert_eq!(DispatchError::MissingTritonApiBase.status(), Some(400));
    assert_eq!(DispatchError::OllamaEmbeddingInput.status(), Some(400));
    assert_eq!(DispatchError::StreamFlag.status(), Some(400));
    assert_eq!(DispatchError::Lemonade(LemonadeError::Shape).status(), None);
    let source = serde_json::from_str::<i32>("x").expect_err("the fixture is not a number");
    assert_eq!(DispatchError::Encode { source }.status(), None);
}

#[test]
fn dispatch_errors_name_what_is_wrong() {
    assert_eq!(
        DispatchError::MissingTritonApiBase.to_string(),
        "api_base is required for triton. Please pass `api_base`"
    );
    assert_eq!(
        DispatchError::OllamaEmbeddingInput.to_string(),
        "Invalid input for ollama embeddings. input must be a string or a list of strings"
    );
    assert_eq!(
        DispatchError::StreamFlag.to_string(),
        "the stream value must be a boolean"
    );
    assert_eq!(
        DispatchError::Lemonade(LemonadeError::Shape).to_string(),
        "the Lemonade models reply is not an object with a data list"
    );
    assert_eq!(
        DispatchError::Lemonade(LemonadeError::Entry).to_string(),
        "a Lemonade model entry holds no string id"
    );
}
