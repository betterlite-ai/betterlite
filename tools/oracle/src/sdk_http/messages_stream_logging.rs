use std::collections::BTreeMap;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::stream::Lease;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use reqwest::Response;
use tracing::subscriber::set_default;

use super::error::SdkHttpError;
use super::messages_header_stream::outbound;
use super::result::SdkResult;
use super::vector_query_embedding::outbound_client;
use call::MessagesStreamLoggingCall;
use outcome::StreamLoggingRun;
use recording::Warnings;
use session::Session;

pub mod call;
mod doubles;
pub mod outcome;
mod recording;
mod session;

struct Http<'call> {
    client: ProxiedClient,
    api_base: &'call str,
}

impl Http<'_> {
    async fn post(&self, body: &IndexMap<String, Json>) -> Result<(Response, Lease), SdkHttpError> {
        let request = outbound(self.api_base, body)?;
        let sent = self
            .client
            .send(request)
            .await
            .map_err(|error| SdkHttpError::Transport(Box::new(error)))?;
        Ok((sent.response, sent.lease))
    }

    async fn read(response: Response) -> Result<Vec<u8>, SdkHttpError> {
        let bytes = response
            .bytes()
            .await
            .map_err(|source| SdkHttpError::Stream(Box::new(HttpError::Request(source))))?;
        Ok(bytes.to_vec())
    }

    async fn frames(&self, body: &IndexMap<String, Json>) -> Result<Vec<Vec<u8>>, SdkHttpError> {
        let (response, _lease) = self.post(body).await?;
        let text =
            String::from_utf8(Self::read(response).await?).map_err(SdkHttpError::FrameUtf8)?;
        Ok(text
            .split_inclusive("\n\n")
            .map(|frame| frame.as_bytes().to_vec())
            .collect())
    }
}

pub(crate) async fn send(
    call: &MessagesStreamLoggingCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let warnings = Warnings::default();
    let writer = warnings.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::WARN)
        .without_time()
        .with_target(false)
        .with_level(false)
        .finish();
    let _guard = set_default(subscriber);
    let http = Http {
        client: outbound_client(env)?,
        api_base,
    };
    let mut session = Session::start(call.body.clone(), warnings)?;
    let mut steps = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        steps.push(session.run(step, &http).await?);
    }
    Ok(SdkResult::MessagesStreamLogging {
        value: StreamLoggingRun { steps },
    })
}
