use crate::foundry_resource_dispatch::{FoundryResourceError, FoundryResourceOperation, dispatch};

#[tokio::test]
async fn foundry_resource_dispatch_unsupported_operation() {
    let result = dispatch(FoundryResourceOperation::Unsupported {
        operation: "videos",
    })
    .await;
    let Err(error) = result else {
        panic!("an operation no foundry family owns is refused");
    };
    let FoundryResourceError::Unsupported { operation } = error else {
        panic!("the refusal names the unsupported operation");
    };
    assert_eq!(operation, "videos");
    assert_eq!(
        FoundryResourceError::Unsupported {
            operation: "videos"
        }
        .to_string(),
        "betterlite does not run the videos operation through the foundry resource provider"
    );
}
