use crate::gemini_live_client::decode::ClientEvent;
// ast-grep-ignore: no-long-names
use crate::gemini_live_client::decode::decode_gemini_live_client_event as decode_client_event;
use crate::gemini_live_client::state::LiveSessionState;
use crate::gemini_live_client::transform::{FunctionResponseId, LiveClient};
use crate::gemini_live_setup::settings::LiveSetupSettings;

use super::error::VertexLiveError;
use super::session::{VertexLiveInput, transform_session_update};

#[derive(Debug)]
pub struct VertexLiveClient {
    pub client: LiveClient,
}

impl Default for VertexLiveClient {
    fn default() -> Self {
        Self {
            // Vertex Live rejects the function-response id that the Studio backend keeps
            client: LiveClient {
                state: LiveSessionState::default(),
                function_response_id: FunctionResponseId::Omit,
            },
        }
    }
}

impl VertexLiveClient {
    pub fn transform_realtime_request(
        &mut self,
        message: &[u8],
        limit: usize,
        input: &VertexLiveInput<'_>,
    ) -> Result<Vec<String>, VertexLiveError> {
        let event = decode_client_event(message, limit)?;
        if let ClientEvent::SessionUpdate(session) = &event {
            return transform_session_update(session.as_ref(), input);
        }
        let settings = LiveSetupSettings {
            catalog: input.catalog,
            schema: input.schema,
            defer_setup: false,
        };
        self.client
            .transform_realtime_request(
                &event,
                input.model,
                input.session_configuration_request,
                &settings,
            )
            .map_err(VertexLiveError::from)
    }
}
