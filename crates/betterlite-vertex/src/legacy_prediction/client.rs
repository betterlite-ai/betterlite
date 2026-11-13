use std::future::Future;
use std::slice;

use betterlite_models::decisions::{Extra, Json};
use futures_util::stream::BoxStream;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Serialize;

use super::error::PredictionError;
use super::records::{LegacyChunk, PredictionsReply, TextReply};
use super::tensor::Tensor;

use self::options::{PredictCall, parameters};

pub mod options;

pub type PredictionStream = BoxStream<'static, Result<LegacyChunk, PredictionError>>;

// The SDK objects the async paths call; a fixture implements this to intercept each operation
pub trait PredictionClient: Send + Sync {
    fn send_message_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> impl Future<Output = Result<TextReply, PredictionError>> + Send;

    fn predict_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> impl Future<Output = Result<TextReply, PredictionError>> + Send;

    fn predict_endpoint(
        &self,
        request: &EndpointPredictRequest,
    ) -> impl Future<Output = Result<PredictionsReply, PredictionError>> + Send;

    fn send_message_streaming_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> impl Future<Output = Result<PredictionStream, PredictionError>> + Send;

    fn predict_streaming_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> impl Future<Output = Result<PredictionStream, PredictionError>> + Send;
}

#[derive(Clone, Debug)]
pub struct PublisherPredictRequest {
    pub token: SecretString,
    pub project: String,
    pub location: String,
    pub model: String,
    pub prompt: String,
    pub options: Extra,
}

#[derive(Clone, Debug)]
pub struct EndpointPredictRequest {
    pub token: SecretString,
    pub location: String,
    pub endpoint: String,
    pub instances: Vec<Json>,
}

#[derive(Serialize)]
struct PredictBody<'body> {
    instances: &'body [Json],
    parameters: &'body Extra,
}

#[derive(Serialize)]
struct EndpointBody<'body> {
    instances: &'body [Json],
}

#[derive(Serialize)]
struct StreamingBody {
    inputs: Vec<Tensor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<Tensor>,
}

pub fn chat_predict_body(prompt: &str, options: &Extra) -> Result<Vec<u8>, PredictionError> {
    let instance = chat_instance(prompt);
    let parameters = parameters(options, PredictCall::Chat)?;
    encode(&PredictBody {
        instances: slice::from_ref(&instance),
        parameters: &parameters,
    })
}

pub fn text_predict_body(prompt: &str, options: &Extra) -> Result<Vec<u8>, PredictionError> {
    let instance = text_instance(prompt);
    let parameters = parameters(options, PredictCall::Text)?;
    encode(&PredictBody {
        instances: slice::from_ref(&instance),
        parameters: &parameters,
    })
}

pub fn chat_streaming_body(prompt: &str, options: &Extra) -> Result<Vec<u8>, PredictionError> {
    let parameters = parameters(options, PredictCall::ChatStreaming)?;
    streaming_body(&chat_instance(prompt), &parameters)
}

pub fn text_streaming_body(prompt: &str, options: &Extra) -> Result<Vec<u8>, PredictionError> {
    let parameters = parameters(options, PredictCall::TextStreaming)?;
    streaming_body(&text_instance(prompt), &parameters)
}

pub fn endpoint_predict_body(instances: &[Json]) -> Result<Vec<u8>, PredictionError> {
    encode(&EndpointBody { instances })
}

fn chat_instance(prompt: &str) -> Json {
    let mut message = IndexMap::new();
    message.insert(String::from("author"), Json::Text(String::from("user")));
    message.insert(String::from("content"), Json::Text(prompt.to_owned()));
    let mut instance = IndexMap::new();
    instance.insert(
        String::from("messages"),
        Json::List(vec![Json::Object(message)]),
    );
    Json::Object(instance)
}

fn text_instance(prompt: &str) -> Json {
    let mut instance = IndexMap::new();
    instance.insert(String::from("content"), Json::Text(prompt.to_owned()));
    Json::Object(instance)
}

fn streaming_body(instance: &Json, parameters: &Extra) -> Result<Vec<u8>, PredictionError> {
    let parameters = if parameters.is_empty() {
        None
    } else {
        Some(Tensor::of_json(&Json::Object(parameters.clone()))?)
    };
    encode(&StreamingBody {
        inputs: vec![Tensor::of_json(instance)?],
        parameters,
    })
}

fn encode<T>(body: &T) -> Result<Vec<u8>, PredictionError>
where
    T: Serialize,
{
    serde_json::to_vec(body).map_err(|source| PredictionError::Serialize { source })
}
