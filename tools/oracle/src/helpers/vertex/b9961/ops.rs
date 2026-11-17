use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use betterlite_vertex::gemini_history::convert::{History, convert_messages_with_history};
use betterlite_vertex::gemini_history::error::HistoryError;
use betterlite_vertex::gemini_media::gcs_metadata::{GcsDeps, GcsTransport, MetadataFuture};
use betterlite_vertex::gemini_request::media_prepare::resolve_gcs_content_types;
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::probe::AccessToken;
use betterlite_vertex::vertex_interactions::mint::{MintAccessToken, MintFuture};
use http::header::{AUTHORIZATION, HeaderMap};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::runtime::Builder;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    messages: Vec<Message>,
    model: String,
    litellm_params: Object,
    // the case scripts one metadata answer for every lookup the conversion performs
    metadata: Metadata,
    mint_error: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    status: u16,
    json: Option<Value>,
    body: Option<String>,
    connect: bool,
}

#[derive(Clone, Debug, Serialize)]
struct RecordedRequest {
    url: String,
    authorization: Option<String>,
}

struct ScriptedGcs {
    status: u16,
    body: Vec<u8>,
    connect: bool,
    requests: Mutex<Vec<RecordedRequest>>,
}

impl ScriptedGcs {
    fn for_script(script: &Metadata) -> Self {
        let body = script.body.clone().map_or_else(
            || {
                script
                    .json
                    .as_ref()
                    .map(Value::to_string)
                    .unwrap_or_default()
                    .into_bytes()
            },
            String::into_bytes,
        );
        Self {
            status: script.status,
            body,
            connect: script.connect,
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl GcsTransport for ScriptedGcs {
    fn get(&self, request: OutboundRequest) -> MetadataFuture<'_> {
        let authorization = request
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedRequest {
                url: request.url.to_string(),
                authorization,
            });
        if self.connect {
            return Box::pin(ready(Err(TransportError::Closed)));
        }
        Box::pin(ready(Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
        })))
    }
}

struct RecordingMint {
    fail: bool,
    mints: Mutex<Vec<Option<String>>>,
}

impl MintAccessToken for RecordingMint {
    fn mint<'mint>(
        &'mint self,
        _credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        self.mints
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(project.map(str::to_owned));
        if self.fail {
            return Box::pin(ready(Err(CredentialCacheError::MissingCredentials)));
        }
        Box::pin(ready(Ok(AccessToken {
            token: SecretString::from("oracle-token"),
            project: project.unwrap_or_default().to_owned(),
        })))
    }
}

// conversion runs after the metadata pass, so every reference it still wants to download was never a gs object
struct NoFetch;

impl RemoteMediaFetcher for NoFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Attempts {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let url = urls.first().cloned().unwrap_or_default();
        ready(Err(ImageFetchError::Attempts { url }))
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = Builder::new_current_thread().build()?;
    let gcs = ScriptedGcs::for_script(&input.metadata);
    let mint = RecordingMint {
        fail: input.mint_error,
        mints: Mutex::new(Vec::new()),
    };
    let deps = GcsDeps {
        transport: &gcs,
        tokens: &mint,
    };
    let resolved = runtime.block_on(resolve_gcs_content_types(
        input.messages.clone(),
        &input.litellm_params,
        &deps,
    ));
    let outcome = match resolved {
        Ok(messages) => runtime.block_on(convert_messages_with_history(
            &messages,
            Some(input.model.as_str()),
            &NoFetch,
        )),
        Err(error) => Err(error),
    };
    Ok(view(&outcome, &gcs, &mint))
}

// a lookup failure travels in the compared value, since the case compares what the converter answers beside the requests and mints it made
fn view(outcome: &Result<History, HistoryError>, gcs: &ScriptedGcs, mint: &RecordingMint) -> Value {
    let requests = gcs
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let mints = mint
        .mints
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    match outcome {
        Ok(history) => json!({
            "requests": requests,
            "mints": mints,
            "contents": history.contents,
            "failure": Value::Null,
        }),
        Err(error) => json!({
            "requests": requests,
            "mints": mints,
            "contents": Value::Null,
            "failure": {
                "status": error.status(),
                "type": error.error_type(),
                "param": error.param(),
                "code": error.code(),
            },
        }),
    }
}
