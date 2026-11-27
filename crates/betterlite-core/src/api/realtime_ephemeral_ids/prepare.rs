use std::error::Error;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::realtime_contracts::records::{RealtimeClientSecretRequest, RealtimeSessionConfig};
use crate::api::realtime_ephemeral_ids::session::{
    DEFAULT_REALTIME_MODEL, DEFAULT_TRANSCRIPTION_MODEL, RealtimeSessionType,
    append_model_candidate, model_candidates, set_transcription_model,
};

// what preparing a session can fail with: encoding the session dump, or the injected authorizer's refusal
#[derive(Debug, thiserror::Error)]
pub enum PrepareError<GrantError: Error + Send + Sync + 'static> {
    #[error("the session could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Grant(#[from] GrantError),
}

// can_key_call_resolved_model's arguments for a session model, injected because the auth reads belong to the caller
pub struct SessionGrantRequest<'request, Router> {
    pub model: &'request str,
    pub llm_model_list: Option<&'request [String]>,
    pub principal: &'request Principal,
    pub router: Option<&'request Router>,
}

// B886's can_key_call_resolved_model, called once per model the client_secrets route may pin
#[async_trait]
pub trait SessionModelGrant<Router> {
    type Error: Error + Send + Sync + 'static;

    async fn authorize(&self, request: &SessionGrantRequest<'_, Router>)
    -> Result<(), Self::Error>;
}

// what every authorization of one session carries besides the model
struct GrantContext<'call, Router> {
    llm_model_list: Option<&'call [String]>,
    principal: &'call Principal,
    router: Option<&'call Router>,
}

// _prepare_client_secret_session's answer: the model that is pinned, the session data to forward, and the coerced type
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedSession {
    pub model: String,
    pub session: Option<IndexMap<String, Json>>,
    pub session_type: RealtimeSessionType,
}

// _prepare_client_secret_session: realtime authorizes one model; transcription authorizes every candidate before rewriting the session to the first
pub async fn prepare_client_secret_session<Router, Grants>(
    request: &RealtimeClientSecretRequest,
    principal: &Principal,
    llm_model_list: Option<&[String]>,
    router: Option<&Router>,
    grants: &Grants,
) -> Result<PreparedSession, PrepareError<Grants::Error>>
where
    Router: Sync,
    Grants: SessionModelGrant<Router> + Sync,
{
    let session_type = RealtimeSessionType::from_request(
        request
            .session
            .as_ref()
            .and_then(|configured| configured.session_type.as_deref()),
    );
    let session_data = session_data_with_type(request.session.as_ref(), session_type)
        .map_err(PrepareError::Encode)?;
    let session_model = request
        .session
        .as_ref()
        .and_then(|configured| configured.model.as_deref())
        .filter(|model| !model.is_empty());
    let root_model = request.model.as_deref().filter(|model| !model.is_empty());
    let context = GrantContext {
        llm_model_list,
        principal,
        router,
    };
    if session_type == RealtimeSessionType::Realtime {
        let model = session_model
            .or(root_model)
            .unwrap_or(DEFAULT_REALTIME_MODEL);
        authorize_model(&context, grants, model).await?;
        return Ok(PreparedSession {
            model: model.to_owned(),
            session: session_data,
            session_type,
        });
    }
    transcription_selection(&context, grants, session_data, session_model, root_model).await
}

// req.session.model_dump(exclude_none=True) with the coerced type written over the client's own
fn session_data_with_type(
    session: Option<&RealtimeSessionConfig>,
    session_type: RealtimeSessionType,
) -> Result<Option<IndexMap<String, Json>>, serde_json::Error> {
    let Some(configured) = session else {
        return Ok(None);
    };
    let encoded = serde_json::to_value(configured)?;
    let mut fields: IndexMap<String, Json> = serde_json::from_value(encoded)?;
    fields.retain(|_, value| !matches!(value, Json::Null));
    fields.insert(
        "type".to_owned(),
        Json::Text(session_type.as_str().to_owned()),
    );
    Ok(Some(fields))
}

// one SessionGrantRequest per model: realtime asks once, transcription once per candidate
async fn authorize_model<Router, Grants>(
    context: &GrantContext<'_, Router>,
    grants: &Grants,
    model: &str,
) -> Result<(), PrepareError<Grants::Error>>
where
    Router: Sync,
    Grants: SessionModelGrant<Router> + Sync,
{
    grants
        .authorize(&SessionGrantRequest {
            model,
            llm_model_list: context.llm_model_list,
            principal: context.principal,
            router: context.router,
        })
        .await?;
    Ok(())
}

// the transcription branch: every candidate is authorized first, then both transcription shapes take the first one and session.model goes away
async fn transcription_selection<Router, Grants>(
    context: &GrantContext<'_, Router>,
    grants: &Grants,
    mut session_data: Option<IndexMap<String, Json>>,
    session_model: Option<&str>,
    root_model: Option<&str>,
) -> Result<PreparedSession, PrepareError<Grants::Error>>
where
    Router: Sync,
    Grants: SessionModelGrant<Router> + Sync,
{
    let (model, candidates) = transcription_plan(session_data.as_ref(), session_model, root_model);
    for candidate in &candidates {
        authorize_model(context, grants, candidate).await?;
    }
    if let Some(fields) = session_data.as_mut() {
        set_transcription_model(fields, &model, true);
        drop(fields.shift_remove("model"));
    }
    Ok(PreparedSession {
        model,
        session: session_data,
        session_type: RealtimeSessionType::Transcription,
    })
}

// the model transcription pins is the first candidate, and every distinct candidate is authorized
#[must_use]
fn transcription_plan(
    session_data: Option<&IndexMap<String, Json>>,
    session_model: Option<&str>,
    root_model: Option<&str>,
) -> (String, Vec<String>) {
    let nested = session_data.map_or_else(Vec::new, model_candidates);
    let first = nested.first().cloned();
    if let Some(model) = first {
        return (model, nested);
    }
    let chosen = session_model
        .or(root_model)
        .unwrap_or(DEFAULT_TRANSCRIPTION_MODEL);
    let mut candidates = vec![chosen.to_owned()];
    append_model_candidate(&mut candidates, session_model);
    append_model_candidate(&mut candidates, root_model);
    (chosen.to_owned(), candidates)
}
