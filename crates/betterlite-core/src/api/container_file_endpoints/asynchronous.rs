use std::sync::Mutex;

use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::containers::container_file_http::ContainerFileReply;

use super::ContainerFileEndpoints;
use super::call::{CallSite, ContainerFileCall};
use super::error::{ContainerFileFailureError, ContainerFileOpError, failure};
use super::handler::{ContainerConfigLookup, ContainerFileHandler, ContainerIdDecoder};
use super::prepare::{Prepared, prepare};

// the async operation's body, which awaits B3363's ahandle instead of the blocking form
pub(super) async fn acontainer_file_call<Handler, Decoder, Configs>(
    endpoints: &ContainerFileEndpoints<'_, Handler, Decoder, Configs>,
    endpoint_name: &str,
    call: &ContainerFileCall,
    logging: &Mutex<LoggingState>,
    default_timeout_seconds: f64,
) -> Result<ContainerFileReply, ContainerFileFailureError>
where
    Handler: ContainerFileHandler,
    Decoder: ContainerIdDecoder,
    Configs: ContainerConfigLookup,
{
    let mut site = CallSite::new(endpoint_name, call, logging, default_timeout_seconds);
    match prepare(
        &endpoints.endpoints,
        &mut site,
        endpoints.decoder,
        endpoints.configs,
    ) {
        Ok(Prepared::Answered(reply)) => Ok(reply),
        Ok(Prepared::Ready(prepared)) => {
            let reply = endpoints
                .handler
                .ahandle(&prepared.handler_call(call, logging))
                .await;
            reply.map_err(|error| {
                failure(
                    ContainerFileOpError::Handler(error),
                    &site.provider,
                    endpoints.redactor,
                )
            })
        }
        Err(error) => Err(failure(error, &site.provider, endpoints.redactor)),
    }
}
