use std::future::ready;

use super::LegacyCompletion;
use super::error::LegacyVertexError;
use super::plan::{LegacyPlan, LegacyRoute, endpoint_path};
use crate::endpoint_target::region::DEFAULT_LOCATION;
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
        stream: streaming,
        ..
    } = plan;
    let invocation = invocation_of(model, prompt, route, context, &access);
    let publisher = cached.unwrap_or(client);
    let chosen = match &invocation {
        LegacyInvocation::Chat { .. } | LegacyInvocation::Text { .. } => publisher,
        LegacyInvocation::CustomEndpoint { .. }
        | LegacyInvocation::PrivateEndpoint { .. }
        | LegacyInvocation::Vision { .. } => client,
    };
    let lookup = ready(Ok::<AccessToken, CredentialCacheError>(access));
    if streaming {
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
) -> LegacyInvocation {
    match route {
        LegacyRoute::Vision => LegacyInvocation::Vision { model, prompt },
        LegacyRoute::Chat { options } => LegacyInvocation::Chat {
            model,
            prompt,
            options,
        },
        LegacyRoute::Text { options, .. } => LegacyInvocation::Text {
            model,
            prompt,
            options,
        },
        LegacyRoute::Custom { instances, options } => LegacyInvocation::CustomEndpoint {
            model,
            prompt,
            instances,
            options,
        },
        LegacyRoute::Private { instances, options } => {
            let endpoint = private_endpoint(context, access, &model);
            LegacyInvocation::PrivateEndpoint {
                model,
                prompt,
                endpoint,
                instances,
                options,
            }
        }
    }
}

// The SDK resolves the private endpoint against the initialized project and location, not the private literal
fn private_endpoint(context: &LegacyContext<'_, '_>, access: &AccessToken, model: &str) -> String {
    let location = context.location.unwrap_or(DEFAULT_LOCATION);
    let project = context.project.unwrap_or(&access.project);
    endpoint_path(project, location, model)
}
