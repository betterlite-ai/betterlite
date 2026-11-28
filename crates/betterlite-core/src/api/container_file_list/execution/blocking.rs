use betterlite_openai::api::container_contracts::records::ContainerFileListResponse;
use betterlite_openai::containers::container_file_http::PreCallHook;
use betterlite_openai::containers::container_file_http::request::read_params;
use betterlite_openai::containers::container_file_http::transport::{
    AsyncClient, BlockingClient, ClientSource, Selected,
};

use super::{ContainerFileListCall, finish, prepare_request, transport_error};
use crate::api::container_file_list::error::FileListHandlerError;

// llm_http_handler's container_file_list_handler, in LiteLLM's order: client, request, send, transform
pub fn container_file_list_handler<Source, Hook>(
    clients: &Source,
    hook: &Hook,
    call: &ContainerFileListCall<'_, Source>,
) -> Result<ContainerFileListResponse, FileListHandlerError>
where
    Source: ClientSource<Blocking: BlockingClient, Asynchronous: AsyncClient>,
    Hook: PreCallHook,
{
    let params = read_params(call.litellm_params)?;
    let client = Selected::choose(call.blocking_client, || {
        clients.blocking_client(params.ssl_verify.as_ref())
    })?;
    let request = prepare_request(hook, call, &params)?;
    let reply = client
        .get()
        .send(&request)
        .map_err(|error| transport_error(call.config, &error))?;
    finish(call.config, &reply)
}
