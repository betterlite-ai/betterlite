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
use self::error::{CallRoute, ContainerFileFailureError, ContainerFileOpError, failure};
use self::handler::{ContainerConfigLookup, ContainerFileHandler, ContainerIdDecoder};
use self::prepare::{Prepared, prepare as prepare_call};

// containers/__init__.py re-exports eight of the ten generated names, leaving out upload_container_file, while the router registers all ten; the name lists below are the whole export surface here
pub struct ContainerFileEndpoints<'deps, Handler, Decoder, Configs> {
    handler: &'deps Handler,
    decoder: &'deps Decoder,
    configs: &'deps Configs,
    redactor: &'deps Redactor,
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

    // endpoint_factory.py's steps to the handler call, with preparation errors mapped under the provider resolved so far
    fn prepared_call(
        &self,
        endpoint_name: &str,
        call: &ContainerFileCall,
        logging: &Mutex<LoggingState>,
        default_timeout_seconds: f64,
    ) -> Result<Prepared<'_>, ContainerFileFailureError> {
        let mut site = CallSite {
            endpoint_name,
            call,
            logging,
            default_timeout_seconds,
            provider: call.custom_llm_provider.clone(),
        };
        prepare_call(&self.endpoints, &mut site, self.decoder, self.configs)
            .map_err(|error| failure(error, &site.provider, self.redactor, None))
    }

    pub fn container_file_call(
        &self,
        endpoint_name: &str,
        call: &ContainerFileCall,
        logging: &Mutex<LoggingState>,
        default_timeout_seconds: f64,
    ) -> Result<ContainerFileReply, ContainerFileFailureError> {
        let prepared =
            match self.prepared_call(endpoint_name, call, logging, default_timeout_seconds)? {
                Prepared::Answered(reply) => return Ok(reply),
                Prepared::Ready(prepared) => prepared,
            };
        self.handler
            .handle(&prepared.handler_call(call, logging))
            .map_err(|error| {
                let route = CallRoute {
                    method: prepared.method,
                    asynchronous: false,
                };
                failure(
                    ContainerFileOpError::Handler(error),
                    &prepared.provider,
                    self.redactor,
                    Some(route),
                )
            })
    }

    // the async twin: LiteLLM maps a handler error on this path with the provider the caller gave
    pub async fn acontainer_file_call(
        &self,
        endpoint_name: &str,
        call: &ContainerFileCall,
        logging: &Mutex<LoggingState>,
        default_timeout_seconds: f64,
    ) -> Result<ContainerFileReply, ContainerFileFailureError> {
        let prepared =
            match self.prepared_call(endpoint_name, call, logging, default_timeout_seconds)? {
                Prepared::Answered(reply) => return Ok(reply),
                Prepared::Ready(prepared) => prepared,
            };
        let reply = self
            .handler
            .ahandle(&prepared.handler_call(call, logging))
            .await;
        reply.map_err(|error| {
            let route = CallRoute {
                method: prepared.method,
                asynchronous: true,
            };
            failure(
                ContainerFileOpError::Handler(error),
                &call.custom_llm_provider,
                self.redactor,
                Some(route),
            )
        })
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
