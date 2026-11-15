use std::future::ready;

use super::LegacyCompletion;
use super::error::LegacyVertexError;
use super::plan::{LegacyPlan, LegacyRoute, endpoint_path};
use crate::legacy_prediction::client::PredictionClient;
use crate::legacy_prediction::execution::LegacyContext;
use crate::legacy_prediction::execution::completion_entry::complete;
use crate::legacy_prediction::execution::streaming_entry::stream;
use crate::legacy_prediction::invocation::LegacyInvocation;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;

pub(crate) async fn delegate<C>(
    client: &C,
    cached: Option<&C>,
    plan: LegacyPlan,
    context: &LegacyContext<'_, '_>,
    access: AccessToken,
) -> Result<LegacyCompletion, LegacyVertexError>
where
    C: PredictionClient,
{
    let LegacyPlan {
        model,
        prompt,
        route,
        stream,
        ..
    } = plan;
    let invocation = invocation_of(model, prompt, route, context, &access)?;
    let publisher = cached.unwrap_or(client);
    let chosen = match &invocation {
        LegacyInvocation::Chat { .. } | LegacyInvocation::Text { .. } => publisher,
        LegacyInvocation::CustomEndpoint { .. }
        | LegacyInvocation::PrivateEndpoint { .. }
        | LegacyInvocation::Vision { .. } => client,
    };
    let lookup = ready(Ok::<AccessToken, CredentialCacheError>(access));
    if stream {
        let chunks = stream(chosen, invocation, context, lookup).await?;
        return Ok(LegacyCompletion::Chunks(chunks));
    }
    let result = complete(chosen, invocation, context, lookup).await?;
    Ok(LegacyCompletion::Response(Box::new(result)))
}

// B1328 runs the asynchronous branches; this builds the invocation the source's selection produced
fn invocation_of(
    model: String,
    prompt: String,
    route: LegacyRoute,
    context: &LegacyContext<'_, '_>,
    access: &AccessToken,
) -> Result<LegacyInvocation, LegacyVertexError> {
    match route {
        LegacyRoute::Vision => Ok(LegacyInvocation::Vision { model, prompt }),
        LegacyRoute::Chat { options } => Ok(LegacyInvocation::Chat {
            model,
            prompt,
            options,
        }),
        LegacyRoute::Text { options, .. } => Ok(LegacyInvocation::Text {
            model,
            prompt,
            options,
        }),
        LegacyRoute::Custom { instances, options } => Ok(LegacyInvocation::CustomEndpoint {
            model,
            prompt,
            instances,
            options,
        }),
        LegacyRoute::Private { instances, options } => {
            let endpoint = private_endpoint(context, access, &model)?;
            Ok(LegacyInvocation::PrivateEndpoint {
                model,
                prompt,
                endpoint,
                instances,
                options,
            })
        }
    }
}

// The SDK resolves the private endpoint against the initialized project and location, not the private literal
fn private_endpoint(
    context: &LegacyContext<'_, '_>,
    access: &AccessToken,
    model: &str,
) -> Result<String, LegacyVertexError> {
    let location = context.location.ok_or(LegacyVertexError::MissingLocation)?;
    let project = context.project.unwrap_or(&access.project);
    Ok(endpoint_path(project, location, model))
}
