use super::plan_cases::{BASE, check_rows};
use crate::bedrock_dispatch_contract::{BedrockDispatchError, BedrockOperation, dispatch};

fn assert_unsupported(operation: &BedrockOperation, name: &str) {
    let error = dispatch(operation).expect_err("a family without an installed operation refuses");
    assert_eq!(
        error.to_string(),
        format!("betterlite does not run the {name} operation through the bedrock provider")
    );
    match error {
        BedrockDispatchError::Unsupported { operation: found } => assert_eq!(found, name),
        BedrockDispatchError::Route(_)
        | BedrockDispatchError::Endpoint(_)
        | BedrockDispatchError::Encode { .. } => {
            panic!("a native family answered a rewritten error");
        }
    }
}

#[test]
fn bedrock_dispatch_contract_unsupported_operation() {
    assert_unsupported(&BedrockOperation::Chat, "chat");
    assert_unsupported(&BedrockOperation::Other, "other endpoints");
    assert_unsupported(&BedrockOperation::Services, "services");
    assert_unsupported(&BedrockOperation::Image, "image generation");
    assert_unsupported(&BedrockOperation::Agent, "agent");
    assert_unsupported(&BedrockOperation::Interactive, "interactive");

    check_rows(&[BASE]);
}
