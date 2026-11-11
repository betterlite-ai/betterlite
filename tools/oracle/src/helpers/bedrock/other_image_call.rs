use std::str;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::image_call::error::ImageCallError;
use betterlite_bedrock::image_call::plan::{ImageCall, ImageEnvironment};
use betterlite_bedrock::image_call::transport::{ClientKind, ImageRequest, SuppliedClient};
use betterlite_bedrock::image_models::params::Params;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::error::RedactionError;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use crate::helpers::{Helper, HelperError, HelperResult};

mod calls;
mod fixture;
mod handlers;
mod tests;

use calls::{Probe, Probes, Supplied};
use fixture::{Client, Factory, Source};
use handlers::{Shape, observe};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    ImageFamilyDispatch(Probes),
    ImageRuntimeUrl(Probes),
    SignedJsonBody(Probes),
    ExtraHeadersOutsideBody(Probes),
    Base64ImageResult(Probes),
    ImageHttpError(Probes),
    ImageTransportSelection(Probes),
}

#[derive(Debug, Error)]
pub(super) enum Failure {
    #[error(transparent)]
    ImageCall(#[from] ImageCallError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Redact(#[from] RedactionError),
    #[error("the probe needs credentials or a bearer token")]
    MissingCredentials,
    #[error("the probe body is not UTF-8")]
    Body(#[from] str::Utf8Error),
    #[error("the prepared value is not an object")]
    Shape,
    #[error("the async runtime could not start")]
    Runtime(#[from] std::io::Error),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let (probes, shape) = match input {
            Input::ImageFamilyDispatch(probes) => (probes, Shape::Dispatch),
            Input::ImageRuntimeUrl(probes) => (probes, Shape::Runtime),
            Input::SignedJsonBody(probes) | Input::ExtraHeadersOutsideBody(probes) => {
                (probes, Shape::Prepare)
            }
            Input::Base64ImageResult(probes) => (probes, Shape::Result),
            Input::ImageHttpError(probes) => (probes, Shape::Error),
            Input::ImageTransportSelection(probes) => (probes, Shape::Selection),
        };
        each(probes, &shape).map_err(Box::<HelperError>::from)
    }
}

fn each(probes: &Probes, shape: &Shape) -> Result<Value, Failure> {
    let mut results = Vec::with_capacity(probes.probes.len());
    for probe in &probes.probes {
        results.push(observe(probe, shape)?);
    }
    Ok(Value::Array(results))
}

pub(crate) struct Harness<'probe> {
    probe: &'probe Probe,
    params: Params,
    extra_headers: Option<Vec<(String, String)>>,
    api_key: Option<SecretString>,
    bearer: Option<SecretString>,
    now: SystemTime,
    pub(crate) source: Source,
    pub(crate) factory: Factory,
    supplied: Option<Client>,
}

impl<'probe> Harness<'probe> {
    pub(crate) fn new(probe: &'probe Probe) -> Result<Self, Failure> {
        if probe.credentials.access_key_id.is_empty()
            && probe.api_key.is_none()
            && probe.configured_bearer_token.is_none()
        {
            return Err(Failure::MissingCredentials);
        }
        let params: Params = serde_json::from_value(Value::Object(probe.optional_params.clone()))?;
        let extra_headers = probe.extra_headers.as_ref().map(|headers| {
            headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        });
        let supplied = match probe.supplied {
            Supplied::None => None,
            Supplied::Sync | Supplied::Async => Some(Client::new(&probe.reply)),
        };
        Ok(Self {
            probe,
            params,
            extra_headers,
            api_key: probe
                .api_key
                .as_ref()
                .map(|key| SecretString::from(key.clone())),
            bearer: probe
                .configured_bearer_token
                .as_ref()
                .map(|token| SecretString::from(token.clone())),
            now: UNIX_EPOCH
                .checked_add(Duration::from_secs(probe.now))
                .ok_or(Failure::Shape)?,
            source: Source::new(probe),
            factory: Factory::new(&probe.reply),
            supplied,
        })
    }

    pub(crate) fn call(&self) -> ImageCall<'_> {
        ImageCall {
            model: &self.probe.model,
            prompt: &self.probe.prompt,
            optional_params: self.params.clone(),
            api_base: self.probe.api_base.as_deref(),
            extra_headers: self.extra_headers.as_deref(),
            api_key: self.api_key.as_ref(),
            timeout: self.probe.timeout_ms.map(Duration::from_millis),
            aimg_generation: self.probe.aimg_generation,
            environment: ImageEnvironment {
                region_name: self.probe.environment.region_name.as_deref(),
                region: self.probe.environment.region.as_deref(),
                session_region: self.probe.environment.session_region.as_deref(),
                runtime_endpoint: self.probe.environment.runtime_endpoint.as_deref(),
                bearer_token: self.bearer.as_ref(),
            },
            now: self.now,
        }
    }

    pub(crate) fn supplied(&self) -> Option<SuppliedClient<'_, Client>> {
        self.supplied.as_ref().map(|client| SuppliedClient {
            kind: match self.probe.supplied {
                Supplied::Async => ClientKind::Async,
                Supplied::None | Supplied::Sync => ClientKind::Sync,
            },
            transport: client,
        })
    }

    pub(crate) fn seen(&self) -> (Vec<ImageRequest>, Vec<ImageRequest>) {
        let supplied = self
            .supplied
            .as_ref()
            .map_or_else(Vec::new, |client| client.seen.requests());
        (supplied, self.factory.seen.requests())
    }
}

impl Failure {
    const fn status(&self) -> Option<u16> {
        match self {
            Self::ImageCall(error) => error.status(),
            Self::Json(_)
            | Self::Logging(_)
            | Self::Redact(_)
            | Self::MissingCredentials
            | Self::Body(_)
            | Self::Shape
            | Self::Runtime(_) => None,
        }
    }
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        Self::new(HelperError {
            class: "ImageCallError".to_owned(),
            status: failure.status(),
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}
