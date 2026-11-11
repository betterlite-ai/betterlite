use crate::azure_dispatch::AzureRequest;
use crate::openai_dispatch::OpenAi;
use crate::operation::{OperationError, OperationRequest};
use crate::provider::Adapter as _;

use super::{catalog, chat, deployment};

#[test]
fn provider_framework_tests_behavior() {
    let operation = OperationRequest::Azure(AzureRequest);
    let refused = OpenAi
        .prepare(&operation, &deployment(), &catalog())
        .expect_err("the openai adapter does not run azure operations");
    if let OperationError::UnsupportedOperation { provider, .. } = refused {
        assert_eq!(provider, "openai");
    } else {
        panic!("the openai adapter answered a different error");
    }
    OpenAi
        .prepare(&chat(), &deployment(), &catalog())
        .expect("the openai adapter prepares its own chat operation");
}
