use std::sync::Arc;

use betterlite_config::api::realtime_connect::RealtimeConnectionSettings;
use betterlite_models::decisions::Json;
use secrecy::SecretString;

use super::super::dispatch::{Forwarding, Prepared, connection_args, empty_json};
use super::super::token::{VertexTokenRefresh, resolve_vertex_token};
use super::super::{
    Branch, RealtimeConnectCall, RealtimeConnectDispatch, RealtimeConnectError, RealtimeConnection,
};
use super::first_text;
use crate::api::vertex_chirp::target::AccessTokenResolver;

struct VertexPlan {
    access_token: SecretString,
    refresh: Arc<dyn AccessTokenResolver>,
    project: String,
    location: Option<String>,
}

async fn vertex_plan<Dispatch>(
    settings: &RealtimeConnectionSettings,
    dispatch: &Dispatch,
    call: &RealtimeConnectCall<'_>,
) -> Result<VertexPlan, RealtimeConnectError<Dispatch::Error>>
where
    Dispatch: RealtimeConnectDispatch,
{
    let credentials = vertex_credentials(settings, call);
    let (project, location) = vertex_placement(settings, call);
    let source = dispatch.vertex_access_tokens()?;
    let (access_token, resolved_project) = resolve_vertex_token(
        source.as_ref(),
        credentials.as_ref(),
        project.as_deref(),
        settings.credential_timeout_seconds,
    )
    .await?;
    // the refresh callback keeps the first call's credentials and the project that call resolved
    let refresh: Arc<dyn AccessTokenResolver> = Arc::new(VertexTokenRefresh {
        source: Arc::clone(&source),
        credentials,
        project: resolved_project.clone(),
        timeout_seconds: settings.credential_timeout_seconds,
    });
    Ok(VertexPlan {
        access_token,
        refresh,
        project: resolved_project,
        location,
    })
}

pub(crate) async fn vertex_connection<Dispatch>(
    settings: &RealtimeConnectionSettings,
    dispatch: &Dispatch,
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
) -> Result<RealtimeConnection<Dispatch::ProviderConfig>, RealtimeConnectError<Dispatch::Error>>
where
    Dispatch: RealtimeConnectDispatch,
{
    let plan = vertex_plan(settings, dispatch, call).await?;
    let api_base = first_text([
        prepared.dynamic_api_base.as_deref(),
        call.params.generic_api_base.as_deref(),
    ]);
    Ok(RealtimeConnection {
        args: connection_args(call, prepared, api_base, None, Forwarding::Provider),
        branch: Branch::Vertex {
            access_token: plan.access_token,
            refresh: plan.refresh,
            project: plan.project,
            location: plan.location,
        },
    })
}

fn vertex_credentials(
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
) -> Option<Json> {
    let named = [
        call.params.vertex_credentials.as_ref(),
        call.params.vertex_ai_credentials.as_ref(),
    ];
    named
        .into_iter()
        .flatten()
        .find(|value| !empty_json(value))
        .cloned()
        .or_else(|| {
            settings
                .vertexai_credentials
                .as_ref()
                .filter(|value| !empty_json(value))
                .cloned()
        })
}

fn vertex_placement(
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
) -> (Option<String>, Option<String>) {
    (
        first_text([
            call.params.vertex_project.as_deref(),
            call.params.vertex_ai_project.as_deref(),
            settings.vertex_project.as_deref(),
            settings.vertexai_project.as_deref(),
        ]),
        first_text([
            call.params.vertex_location.as_deref(),
            call.params.vertex_ai_location.as_deref(),
            settings.vertex_location.as_deref(),
            settings.vertexai_location.as_deref(),
        ]),
    )
}
