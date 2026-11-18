use std::sync::Arc;

use betterlite_openai::watsonx_audio::error::api_status;
use betterlite_openai::watsonx_auth::WatsonxAuth;
use betterlite_openai::watsonx_chat::error::WatsonxChatError;
use betterlite_provider::enterprise_ml_dispatch::transport::SharedTransport;
use betterlite_provider::enterprise_ml_dispatch::watsonx;

use super::fixture::Fixture;
use super::wire::{Failure, failure};

pub(super) mod sagemaker;
pub(super) mod sap;
pub(super) mod watsonx;

pub(super) use sagemaker::sagemaker_chat;
pub(super) use sap::sap_chat;
pub(super) use watsonx::{watsonx_chat, watsonx_text};

// only the API failures among the chat handler's errors carry a status, the field and JSON errors carry none
pub(super) fn chat_failure(error: WatsonxChatError) -> Failure {
    let status = match &error {
        WatsonxChatError::Auth(inner) => api_status(inner),
        WatsonxChatError::InvalidField { .. } | WatsonxChatError::Json(_) => None,
    };
    failure(status, &error)
}

// the IAM auth both watsonx arms build, over the case environment and the fixture transport
pub(super) fn watsonx_auth(fixture: &Fixture) -> Result<WatsonxAuth<SharedTransport<'_>>, Failure> {
    let environment =
        watsonx::environment(&fixture.source()).map_err(|error| failure(error.status(), &error))?;
    let transport = SharedTransport {
        inner: &fixture.transport,
    };
    WatsonxAuth::new(environment, transport, Arc::new(|| 1_700_000_000.0_f64))
        .map_err(|error| failure(api_status(&error), &error))
}
