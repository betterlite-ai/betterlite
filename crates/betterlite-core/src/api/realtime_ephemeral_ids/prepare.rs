use std::error::Error;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::realtime_contracts::records::{
    MaxOutputTokens, RealtimeClientSecretRequest, RealtimeSessionConfig,
};
use crate::api::realtime_ephemeral_ids::session::{
    DEFAULT_REALTIME_MODEL, DEFAULT_TRANSCRIPTION_MODEL, RealtimeSessionType,
    append_model_candidate, model_candidates, set_transcription_model,
};

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
) -> Result<PreparedSession, Grants::Error>
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
    let session_data = session_data_with_type(request.session.as_ref(), session_type);
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
) -> Option<IndexMap<String, Json>> {
    let mut fields = session.map(session_fields)?;
    fields.insert(
        "type".to_owned(),
        Json::Text(session_type.as_str().to_owned()),
    );
    Some(fields)
}

// one SessionGrantRequest per model: realtime asks once, transcription once per candidate
async fn authorize_model<Router, Grants>(
    context: &GrantContext<'_, Router>,
    grants: &Grants,
    model: &str,
) -> Result<(), Grants::Error>
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
        .await
}

// the transcription branch: every candidate is authorized first, then both transcription shapes take the first one and session.model goes away
async fn transcription_selection<Router, Grants>(
    context: &GrantContext<'_, Router>,
    grants: &Grants,
    mut session_data: Option<IndexMap<String, Json>>,
    session_model: Option<&str>,
    root_model: Option<&str>,
) -> Result<PreparedSession, Grants::Error>
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

// RealtimeSessionConfig.model_dump(exclude_none=True): the declared fields in declaration order, then the extras, with None dropped
fn session_fields(session: &RealtimeSessionConfig) -> IndexMap<String, Json> {
    let mut fields = IndexMap::new();
    insert_text(&mut fields, "type", session.session_type.as_deref());
    insert_text(&mut fields, "model", session.model.as_deref());
    insert_text(&mut fields, "instructions", session.instructions.as_deref());
    insert_object(&mut fields, "audio", session.audio.as_ref());
    insert_text_list(&mut fields, "include", session.include.as_deref());
    insert_max_output_tokens(&mut fields, session.max_output_tokens.as_ref());
    insert_text_list(
        &mut fields,
        "output_modalities",
        session.output_modalities.as_deref(),
    );
    insert_value(&mut fields, "tool_choice", session.tool_choice.as_ref());
    if let Some(tools) = session.tools.as_deref() {
        let items: Vec<Json> = tools.iter().cloned().map(Json::Object).collect();
        fields.insert("tools".to_owned(), Json::List(items));
    }
    insert_value(&mut fields, "tracing", session.tracing.as_ref());
    insert_value(&mut fields, "truncation", session.truncation.as_ref());
    insert_object(&mut fields, "prompt", session.prompt.as_ref());
    for (key, value) in &session.extra {
        if !matches!(value, Json::Null) {
            fields.insert(key.clone(), value.clone());
        }
    }
    fields
}

fn insert_text(fields: &mut IndexMap<String, Json>, key: &str, value: Option<&str>) {
    if let Some(text) = value {
        fields.insert(key.to_owned(), Json::Text(text.to_owned()));
    }
}

fn insert_object(
    fields: &mut IndexMap<String, Json>,
    key: &str,
    value: Option<&IndexMap<String, Json>>,
) {
    if let Some(object) = value {
        fields.insert(key.to_owned(), Json::Object(object.clone()));
    }
}

fn insert_text_list(fields: &mut IndexMap<String, Json>, key: &str, values: Option<&[String]>) {
    if let Some(items) = values {
        let list: Vec<Json> = items.iter().cloned().map(Json::Text).collect();
        fields.insert(key.to_owned(), Json::List(list));
    }
}

fn insert_max_output_tokens(fields: &mut IndexMap<String, Json>, value: Option<&MaxOutputTokens>) {
    if let Some(tokens) = value {
        let count = match tokens {
            MaxOutputTokens::Count(count) => Json::Number(Number::from(*count)),
            MaxOutputTokens::Text(text) => Json::Text(text.clone()),
        };
        fields.insert("max_output_tokens".to_owned(), count);
    }
}

fn insert_value(fields: &mut IndexMap<String, Json>, key: &str, value: Option<&Json>) {
    if let Some(item) = value {
        fields.insert(key.to_owned(), item.clone());
    }
}
