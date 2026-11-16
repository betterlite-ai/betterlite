mod asynchronous;
pub mod call;
pub mod error;
pub mod handler;
mod prepare;
mod tests;

use std::sync::Mutex;

use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::api::container_contracts::catalog::{
    CatalogError, ContainerEndpoint, container_endpoints,
};
use betterlite_openai::containers::container_file_http::ContainerFileReply;
use betterlite_redact::secret::Redactor;

use self::call::{CallSite, ContainerFileCall};
use self::error::{ContainerFileFailureError, ContainerFileOpError, failure};
use self::handler::{ContainerConfigLookup, ContainerFileHandler, ContainerIdDecoder};
use self::prepare::{Prepared, prepare as prepare_call};

// containers/__init__.py re-exports eight of the ten generated names, leaving out upload_container_file, while the router registers all ten; the name lists below are the whole export surface here
pub struct ContainerFileEndpoints<'deps, Handler, Decoder, Configs> {
    pub handler: &'deps Handler,
    pub decoder: &'deps Decoder,
    pub configs: &'deps Configs,
    pub redactor: &'deps Redactor,
    endpoints: Vec<ContainerEndpoint>,
}

impl<'deps, Handler, Decoder, Configs> ContainerFileEndpoints<'deps, Handler, Decoder, Configs>
where
    Handler: ContainerFileHandler,
    Decoder: ContainerIdDecoder,
    Configs: ContainerConfigLookup,
{
    pub fn new(
        handler: &'deps Handler,
        decoder: &'deps Decoder,
        configs: &'deps Configs,
        redactor: &'deps Redactor,
    ) -> Result<Self, CatalogError> {
        container_endpoints().map(|endpoints| Self {
            handler,
            decoder,
            configs,
            redactor,
            endpoints,
        })
    }

    pub fn container_file_call(
        &self,
        endpoint_name: &str,
        call: &ContainerFileCall,
        logging: &Mutex<LoggingState>,
        default_timeout_seconds: f64,
    ) -> Result<ContainerFileReply, ContainerFileFailureError> {
        let mut site = CallSite::new(endpoint_name, call, logging, default_timeout_seconds);
        match prepare_call(&self.endpoints, &mut site, self.decoder, self.configs) {
            Ok(Prepared::Answered(reply)) => Ok(reply),
            Ok(Prepared::Ready(prepared)) => {
                let reply = self.handler.handle(&prepared.handler_call(call, logging));
                reply.map_err(|error| {
                    failure(
                        ContainerFileOpError::Handler(error),
                        &site.provider,
                        self.redactor,
                    )
                })
            }
            Err(error) => Err(failure(error, &site.provider, self.redactor)),
        }
    }

    pub async fn acontainer_file_call(
        &self,
        endpoint_name: &str,
        call: &ContainerFileCall,
        logging: &Mutex<LoggingState>,
        default_timeout_seconds: f64,
    ) -> Result<ContainerFileReply, ContainerFileFailureError> {
        asynchronous::acontainer_file_call(
            self,
            endpoint_name,
            call,
            logging,
            default_timeout_seconds,
        )
        .await
    }
}

// generate_container_endpoints' name table: each catalog entry's name, then its async_name
pub fn all_endpoint_names() -> Result<Vec<String>, CatalogError> {
    Ok(container_endpoints()?
        .into_iter()
        .flat_map(|entry| [entry.name, entry.async_name])
        .collect())
}

pub fn async_endpoint_names() -> Result<Vec<String>, CatalogError> {
    Ok(container_endpoints()?
        .into_iter()
        .map(|entry| entry.async_name)
        .collect())
}
