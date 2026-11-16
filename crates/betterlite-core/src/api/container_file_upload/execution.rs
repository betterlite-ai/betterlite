use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::future::Future;
use std::sync::Mutex;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::api::container_contracts::records::ContainerFileObject;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::containers::container_file_http::transport::TransferError;
use betterlite_openai::containers::container_file_http::{
    ContainerFileHttpError, ContainerFileUpload,
};
use indexmap::IndexMap;

use crate::containers::container_request_parameters::ContainerProviderConfig;

// One upload as B3363's handler takes it: the provider is named, and the caller holding the credentials builds its config
pub struct ContainerFileUploadCall<'call> {
    pub endpoint_name: &'static str,
    pub provider_config: ContainerProviderConfig,
    pub litellm_params: &'call IndexMap<String, JsonValue>,
    pub logging: &'call Mutex<LoggingState>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub extra_query: Option<&'call IndexMap<String, ParamValue>>,
    // LiteLLM's timeout argument after `timeout or DEFAULT_REQUEST_TIMEOUT`; seconds, as the argument declares
    pub timeout_seconds: f64,
    pub container_id: &'call str,
    pub file: &'call ContainerFileUpload,
}

// what exception_type reads off the handler's failure: the message, and the status a provider answer or a timeout carried
pub trait UploadFailure: StdError + Send + Sync + 'static {
    fn status_code(&self) -> Option<u16>;

    fn message(&self) -> String;
}

// B3363's GenericContainerHandler::handle
pub trait UploadExecutor: Send + Sync {
    type Failure: UploadFailure;

    fn upload(
        &self,
        call: &ContainerFileUploadCall<'_>,
    ) -> Result<ContainerFileObject, Self::Failure>;
}

// B3363's GenericContainerHandler::ahandle
pub trait AsyncUploadExecutor: Send + Sync {
    type Failure: UploadFailure;

    fn aupload(
        &self,
        call: &ContainerFileUploadCall<'_>,
    ) -> impl Future<Output = Result<ContainerFileObject, Self::Failure>> + Send;
}

// an httpx timeout becomes litellm.Timeout with status 408; every other handler failure reaches the catchall with no status
impl UploadFailure for ContainerFileHttpError {
    fn status_code(&self) -> Option<u16> {
        if let Self::Provider(error) = self {
            return Some(error.status_code);
        }
        if let Self::Transfer(TransferError::Transport(
            TransportError::Deadline(_) | TransportError::ConnectTimeout(_),
        )) = self
        {
            return Some(408);
        }
        None
    }

    fn message(&self) -> String {
        self.to_string()
    }
}
