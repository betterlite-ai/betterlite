use super::convert::{ModerationCallWire, OPENAI_PROVIDER, moderation_model};
use super::error::{CallFacts, CallFailure, moderation_failure};
use super::session::{Entry, TextSession, secret_text};
use betterlite_openai::moderation_execution::send;
use betterlite_openai::moderation_request::ModerationRequest;
use betterlite_openai::moderation_response::ModerationResponse;
use indexmap::IndexMap;

pub(crate) async fn moderate(
    session: &TextSession,
    wire: ModerationCallWire,
    entry: Entry,
) -> Result<ModerationResponse, CallFailure> {
    let ModerationCallWire {
        arguments,
        settings,
    } = wire;
    let api_key = secret_text(arguments.api_key.as_ref())
        .or_else(|| secret_text(settings.api_key.as_ref()))
        .or_else(|| secret_text(settings.openai_key.as_ref()));
    let model = moderation_model(&arguments, entry);
    let input = arguments.input;
    let deployment = session.moderation_deployment(&model, api_key, arguments.api_base.clone())?;
    let executor = session.chat_executor(&deployment)?;
    let request = ModerationRequest {
        model: Some(model.clone()),
        input,
        passthrough: IndexMap::new(),
    };
    let facts = CallFacts {
        model: &model,
        provider: OPENAI_PROVIDER,
    };
    let outcome = send(&executor, &deployment, &request)
        .await
        .map_err(|error| moderation_failure(&error, &facts, &session.redactor))?;
    Ok(outcome.response)
}
