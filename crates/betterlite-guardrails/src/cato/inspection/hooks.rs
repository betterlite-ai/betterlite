use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use tokio::sync::mpsc;

use super::{CallerArgs, call_id};
use crate::cato::error::CatoError;
use crate::cato::headers::build_cato_headers;
use crate::cato::transport::stream::relay_output_stream;
use crate::cato::transport::websocket::WebSocketConnector;
use crate::cato::{CallHook, CatoGuardrail};

// LiteLLM's async_pre_call_hook and async_moderation_hook differ only in the hook header they send
macro_rules! request_hook {
    ($name:ident, $hook:expr) => {
        pub async fn $name(
            guardrail: &CatoGuardrail,
            data: &mut BTreeMap<String, GuardrailValue>,
            call_type: &str,
            caller: CallerArgs<'_>,
        ) -> Result<(), CatoError> {
            if skips_non_conversational(guardrail, call_type) {
                tracing::debug!(call_type, "Cato: skipping non-conversational call type");
                return Ok(());
            }
            guardrail
                .call(data, $hook, caller.key_alias, caller.user_email)
                .await
        }
    };
}

request_hook!(pre_call_hook, CallHook::PreCall);
request_hook!(moderation_hook, CallHook::Moderation);

// embeddings carry documents being indexed, not a conversation to inspect
fn skips_non_conversational(guardrail: &CatoGuardrail, call_type: &str) -> bool {
    matches!(call_type, "embedding" | "aembedding") && !guardrail.inspect_embeddings
}

// LiteLLM's async_post_call_streaming_iterator_hook: relay the model stream through Cato's output analyzer
pub async fn streaming_iterator_hook(
    guardrail: &CatoGuardrail,
    request_data: &BTreeMap<String, GuardrailValue>,
    caller: CallerArgs<'_>,
    response: mpsc::Receiver<GuardrailValue>,
    output: mpsc::Sender<GuardrailValue>,
) -> Result<(), CatoError> {
    let headers = build_cato_headers(
        guardrail,
        CallHook::Output,
        caller.key_alias,
        caller.user_email,
        call_id(request_data),
    )?;
    relay_output_stream(
        &WebSocketConnector::from(guardrail),
        guardrail,
        headers,
        response,
        &output,
    )
    .await
}
