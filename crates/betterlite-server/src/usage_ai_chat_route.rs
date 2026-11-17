pub mod body;
pub mod caller;
pub mod state;

mod error;
mod stream;
mod tests;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::response::Response;
use axum::routing::post;

use crate::auth::auth_identity::AuthContext;
use crate::keepalive_interval::coerce_keepalive_interval;

use self::body::decode_body;
use self::error::UsageAiChatError;
use self::state::{UsageAiChatState, UsageChatFactory};

const USAGE_AI_CHAT_PATH: &str = "/usage/ai/chat";

pub fn routes<F>(state: UsageAiChatState<F>) -> Router
where
    F: UsageChatFactory + 'static,
{
    Router::new()
        .route(USAGE_AI_CHAT_PATH, post(usage_ai_chat::<F>))
        .with_state(state)
}

async fn usage_ai_chat<F>(
    State(state): State<UsageAiChatState<F>>,
    auth: Option<Extension<AuthContext>>,
    payload: Bytes,
) -> Result<Response, UsageAiChatError>
where
    F: UsageChatFactory + 'static,
{
    let Some(Extension(context)) = auth else {
        return Err(UsageAiChatError::Unauthenticated);
    };
    let request = decode_body(&payload)?;
    let caller = caller::identity(&context.principal, state.caller_id.as_ref())?;
    let parts = state.factory.parts();
    let interval = coerce_keepalive_interval(state.ping_interval.as_ref());
    let frames = stream::frame_stream(parts, request, caller.user_id, caller.is_admin, interval);
    Ok(stream::sse_response(frames))
}
