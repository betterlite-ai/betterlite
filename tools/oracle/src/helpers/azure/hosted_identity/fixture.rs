use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::Mutex;

use betterlite_azure::azure_arc_identity::{ArcHost, ArcPlatform};
use betterlite_azure::azure_client_secret::transport::{Clock, HttpMethod};
use betterlite_azure::azure_hosted_identity::request::{
    HostedRequest, HostedResponse, HostedTransport,
};
use betterlite_config::sections::azure_arc_identity::Settings;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::json;

#[derive(Clone, Deserialize)]
pub(crate) struct Reply {
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: String,
}

pub(super) struct Recording {
    replies: Mutex<VecDeque<Reply>>,
    pub(super) requests: Mutex<Vec<serde_json::Value>>,
}

impl Recording {
    pub(super) fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies.to_vec())),
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl HostedTransport for Recording {
    fn send(&self, request: &HostedRequest) -> io::Result<HostedResponse> {
        let headers: BTreeMap<String, String> = request
            .headers
            .iter()
            .map(|(name, value)| (name.to_lowercase(), value.expose_secret().to_owned()))
            .collect();
        self.requests
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .push(json!({
                "method": match request.method {
                    HttpMethod::Get => "GET",
                    HttpMethod::Post => "POST",
                },
                "url": request.url,
                "params": BTreeMap::from_iter(request.query.iter().cloned()),
                "headers": headers,
                "form": (!request.form.is_empty())
                    .then(|| BTreeMap::from_iter(request.form.iter().cloned())),
            }));
        let reply = self
            .replies
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .pop_front()
            .ok_or_else(|| io::Error::other("no reply queued"))?;
        Ok(HostedResponse {
            content_type: reply
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                .map(|(_, value)| value.clone()),
            body: SecretString::from(reply.body),
        })
    }
}

pub(super) struct FixedClock {
    pub(super) now: i64,
}

impl Clock for FixedClock {
    fn now_unix_seconds(&self) -> i64 {
        self.now
    }
}

pub(super) struct Selection<'step> {
    pub(super) environment: &'step BTreeMap<String, String>,
    pub(super) platform: Option<&'step str>,
    pub(super) files: &'step BTreeMap<String, String>,
    pub(super) settings: Settings,
}

impl ArcHost for Selection<'_> {
    fn platform(&self) -> ArcPlatform {
        match self.platform {
            Some("win32") => ArcPlatform::Windows,
            None | Some("" | "linux") => ArcPlatform::Linux,
            Some(other) => ArcPlatform::Other(other.to_owned()),
        }
    }

    fn environment(&self, name: &str) -> Option<String> {
        self.environment.get(name).cloned()
    }

    fn program_data(&self) -> String {
        self.settings.program_data.clone()
    }

    fn program_files(&self) -> String {
        self.settings.program_files.clone()
    }

    fn file_size(&self, path: &str) -> io::Result<u64> {
        u64::try_from(self.read_text(path)?.len()).map_err(io::Error::other)
    }

    fn read_text(&self, path: &str) -> io::Result<String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.to_owned()))
    }

    fn now_unix_seconds(&self) -> i64 {
        0
    }
}
