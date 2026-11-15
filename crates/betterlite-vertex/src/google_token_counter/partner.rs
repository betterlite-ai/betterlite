use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_models::decisions::Json;

use crate::google_token_counter::counter::CountTokensCall;
use crate::google_token_counter::error::CountTokensError;
use crate::google_token_counter::reply::{CountReply, TokenCountResponse};
use crate::partner_count::call::{PartnerCountOverrides, PartnerCountRequest, count_tokens};
use crate::partner_count::execute::PartnerCountTransport;
use crate::partner_count::options::PartnerCountOptions;
use crate::vertex_interactions::mint::MintAccessToken;

// LiteLLM's partner branch: B1332 sends the messages as given and the explicit aliases overwrite the deployment copy
pub(crate) async fn count<T>(
    transport: &T,
    mint: &dyn MintAccessToken,
    settings: &VertexSettings,
    call: &CountTokensCall<'_>,
) -> Result<TokenCountResponse, CountTokensError>
where
    T: PartnerCountTransport,
{
    let messages = encoded_messages(call)?;
    let options = PartnerCountOptions {
        vertex_count_tokens_location: call
            .deployment
            .vertex_count_tokens_location
            .map(str::to_owned),
    };
    let request = PartnerCountRequest {
        model: call.model_to_use,
        messages: &messages,
        params: call.deployment.vertex,
        options: &options,
        settings,
        api_base: call.deployment.api_base,
        overrides: PartnerCountOverrides {
            project: call.deployment.explicit_project(),
            location: call.deployment.explicit_location(),
            credentials: call.deployment.explicit_credentials(),
        },
    };
    let reply = count_tokens(transport, &request, mint).await?;
    let tokenizer_type = reply.tokenizer_used.to_owned();
    Ok(TokenCountResponse::new(
        call,
        reply.input_tokens,
        tokenizer_type,
        CountReply::Partner(reply),
    ))
}

fn encoded_messages(call: &CountTokensCall<'_>) -> Result<Vec<Json>, CountTokensError> {
    let mut encoded = Vec::with_capacity(call.messages.len());
    for message in call.messages {
        let value = serde_json::to_value(message)
            .and_then(serde_json::from_value::<Json>)
            .map_err(|source| CountTokensError::Message { source })?;
        encoded.push(value);
    }
    Ok(encoded)
}
