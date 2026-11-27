use betterlite_bedrock::image_call::plan::{CredentialSource, LogContext};
use betterlite_bedrock::image_call::transport::ClientFactory;

use crate::operation::{OperationRequest, OperationResult};

use super::call::{BedrockImageOperation, Transport};
use super::error::BedrockImageError;
use super::{BedrockImageRequest, BedrockImageResult};

mod edit;
mod generate;

use self::edit::edit;
use self::generate::generate;

// The closed provider enum is the only entry: a request for another arm is refused rather than guessed
pub async fn dispatch<Source, Factory>(
    request: OperationRequest,
    operation: BedrockImageOperation<'_, '_, '_>,
    source: &Source,
    transport: Transport<'_, Factory>,
    log: &mut LogContext<'_, '_>,
) -> Result<OperationResult, BedrockImageError>
where
    Source: CredentialSource + Sync,
    Factory: ClientFactory + Sync,
{
    let OperationRequest::BedrockImage(BedrockImageRequest) = request else {
        return Err(BedrockImageError::UnsupportedOperation);
    };
    let result = match operation {
        BedrockImageOperation::Generate(generate_operation) => BedrockImageResult::Generate(
            Box::new(generate(&generate_operation, source, transport, log).await?),
        ),
        BedrockImageOperation::Edit(edit_operation) => BedrockImageResult::Edit(Box::new(
            edit(&edit_operation, source, transport, log).await?,
        )),
        BedrockImageOperation::Unsupported { operation: name } => {
            return Err(BedrockImageError::Unsupported { operation: name });
        }
    };
    Ok(OperationResult::BedrockImage(result))
}
