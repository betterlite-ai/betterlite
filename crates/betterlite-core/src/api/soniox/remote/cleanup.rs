use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;

use super::SonioxRemote;
use super::exchange::{endpoint_url, header_map};
use super::transport::{RemoteTransport, Sleeper};

pub const TRANSCRIPTION_SELECTION: &str = "transcription";
pub const FILE_SELECTION: &str = "file";
pub const TRANSCRIPTIONS_PATH: &str = "/v1/transcriptions";
pub const FILES_PATH: &str = "/v1/files";

impl<T: RemoteTransport, S: Sleeper> SonioxRemote<'_, T, S> {
    // Best-effort: a failed delete leaves stale data on Soniox but never replaces the transcription result.
    pub async fn cleanup(&self, transcription_id: Option<&str>, file_id: Option<&str>) {
        let selected = &self.prepared.options.cleanup;
        if selected.is_empty() {
            return;
        }
        if selected.iter().any(|name| name == TRANSCRIPTION_SELECTION)
            && let Some(id) = transcription_id.filter(|id| !id.is_empty())
        {
            self.delete(&format!("{TRANSCRIPTIONS_PATH}/{id}")).await;
        }
        if selected.iter().any(|name| name == FILE_SELECTION)
            && let Some(id) = file_id.filter(|id| !id.is_empty())
        {
            self.delete(&format!("{FILES_PATH}/{id}")).await;
        }
    }

    async fn delete(&self, path: &str) {
        let Ok(url) = endpoint_url(&format!("{}{path}", self.prepared.api_base)) else {
            return;
        };
        let Ok(headers) = header_map(&self.prepared.auth_headers) else {
            return;
        };
        let request = OutboundRequest {
            method: Method::DELETE,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Empty,
            deadline: self.timeout,
        };
        let _ignored = self.transport.call(request).await;
    }
}
