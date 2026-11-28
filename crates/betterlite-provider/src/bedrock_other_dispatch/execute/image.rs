use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock::image_call::plan::LogContext as ImageLogContext;
use betterlite_bedrock::image_call::transport::ClientFactory as ImageClientFactory;

use crate::bedrock_image_dispatch::BedrockImageRequest;
use crate::bedrock_image_dispatch::call::{BedrockImageOperation, Transport as ImageTransport};
use crate::bedrock_image_dispatch::error::BedrockImageError;
use crate::bedrock_image_dispatch::execute::dispatch as dispatch_image;
use crate::operation::{OperationRequest, OperationResult};

use super::{LogContext, Services};

// B679's generation and edit composition, reached through this dispatcher's own arm and result
pub(super) async fn image<Source, Factory, Transport, Files>(
    operation: BedrockImageOperation<'_, '_, '_>,
    services: &Services<'_, Source, Factory, Transport, Files>,
    log: &mut LogContext<'_, '_>,
) -> Result<OperationResult, BedrockImageError>
where
    Source: CredentialSource,
    Factory: ImageClientFactory + Sync,
    Transport: Sync,
    Files: Sync,
{
    let mut family_log = ImageLogContext {
        state: &mut *log.state,
        env: log.env,
    };
    dispatch_image(
        OperationRequest::BedrockImage(BedrockImageRequest),
        operation,
        services.source,
        ImageTransport {
            factory: services.factory,
            supplied: None,
        },
        &mut family_log,
    )
    .await
}
