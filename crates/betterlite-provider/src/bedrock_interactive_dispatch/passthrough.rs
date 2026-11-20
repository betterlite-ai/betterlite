use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use betterlite_bedrock::passthrough::BedrockPassthroughPlan;
use betterlite_bedrock::passthrough::error::PassthroughFailure;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughError, PassthroughLogging, PassthroughSigning, PassthroughTarget,
    PassthroughUrlRequest, RelayedResponse,
};
use http::Method;

use super::http::{LeafReply, LeafRequest, LeafTransport};

// A native passthrough call: the leaf plan owns URL, signing and the logging collector
pub struct Operation<'call> {
    pub plan: BedrockPassthroughPlan<'call>,
    pub url_request: PassthroughUrlRequest<'call>,
    pub signing: PassthroughSigning<'call>,
    pub headers: Headers,
    pub method: Method,
    pub deadline: Duration,
    pub logging: &'call mut (dyn PassthroughLogging + Send),
}

#[derive(Debug)]
pub struct Outcome {
    pub request: LeafRequest,
    pub endpoint_url: String,
    pub status: u16,
    pub streaming: bool,
    pub logged: Option<Json>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Leaf(#[from] PassthroughFailure),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("the streamed Bedrock reply could not be logged")]
    Collector {
        #[source]
        source: PassthroughError,
    },
}

pub type RunFuture<'call> = Pin<Box<dyn Future<Output = Result<Outcome, Error>> + Send + 'call>>;

impl Operation<'_> {
    pub async fn run(self, transport: &dyn LeafTransport) -> Result<Outcome, Error> {
        let target = self.url_request.target;
        let (url, endpoint_url) = self.plan.get_complete_url(&self.url_request)?;
        let empty = Object::new();
        let data = self.signing.request_data.unwrap_or(&empty);
        let streaming = self.plan.is_streaming_request(target.endpoint, data);
        let (signed_headers, body) = self
            .plan
            .sign_request(self.headers.clone(), &self.signing)?;
        let request = LeafRequest {
            method: self.method.as_str().to_owned(),
            url: url.to_string(),
            headers: signed_headers
                .into_iter()
                .map(|(name, value)| (name, Some(value)))
                .collect(),
            body: body.unwrap_or_default(),
            deadline: self.deadline,
        };
        let reply = transport.send(request.clone()).await?;
        let logged = if streaming {
            let mut collector = self.plan.create_stream_collector(&target);
            for chunk in &reply.chunks {
                collector.add(chunk);
            }
            collector
                .build_logged_response(self.logging)
                .map_err(|source| Error::Collector { source })?
        } else {
            buffered(&self.plan, target, &self.signing, &reply, self.logging)
        };
        Ok(Outcome {
            request,
            endpoint_url,
            status: reply.status,
            streaming,
            logged,
        })
    }
}

fn buffered(
    plan: &BedrockPassthroughPlan<'_>,
    target: PassthroughTarget<'_>,
    signing: &PassthroughSigning<'_>,
    reply: &LeafReply,
    logging: &mut dyn PassthroughLogging,
) -> Option<Json> {
    let empty = Object::new();
    let data = signing.request_data.unwrap_or(&empty);
    plan.logging_non_streaming_response(
        &target,
        &RelayedResponse {
            status_code: reply.status,
            body: &reply.body,
        },
        data,
        logging,
    )
}
