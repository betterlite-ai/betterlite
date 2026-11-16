use crate::openai_resource_dispatch::{OpenAiResourceError, OpenAiResourceOperation, dispatch};

#[tokio::test]
async fn openai_resource_dispatch_unsupported_operation() {
    let result = dispatch(OpenAiResourceOperation::Unsupported {
        operation: "images",
    })
    .await;
    let Err(error) = result else {
        panic!("an operation no family owns is refused");
    };
    let OpenAiResourceError::Unsupported { operation } = error else {
        panic!("the refusal names the unsupported operation");
    };
    assert_eq!(operation, "images");
    assert_eq!(
        OpenAiResourceError::Unsupported {
            operation: "images"
        }
        .to_string(),
        "betterlite does not run the images operation through the openai resource provider"
    );
}
