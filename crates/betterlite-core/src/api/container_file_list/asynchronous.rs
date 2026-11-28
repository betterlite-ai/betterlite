use betterlite_openai::api::container_contracts::records::ContainerFileListResponse;
use betterlite_openai::containers::container_file_http::PreCallHook;
use betterlite_openai::containers::container_file_http::transport::{
    AsyncClient, BlockingClient, ClientSource,
};

use super::error::{ContainerFileListError, ContainerFileListFailure, failure};
// ast-grep-ignore: no-long-names
use super::execution::async_container_file_list_handler;
use super::prepare::{Prepared, prepare_call};
use super::{ContainerFileListDeps, ContainerFileListRequest, caller_defaults};
use crate::api::container_file_endpoints::handler::ContainerConfigLookup;

// LiteLLM's alist_container_files: mock first, then the handler runs the call it prepared
pub async fn alist_container_files<Source, Hook, Configs>(
    clients: &Source,
    hook: &Hook,
    configs: &Configs,
    deps: ContainerFileListDeps<'_>,
    request: &ContainerFileListRequest<'_>,
) -> Result<ContainerFileListResponse, ContainerFileListFailure>
where
    Source: ClientSource<Blocking: BlockingClient, Asynchronous: AsyncClient>,
    Hook: PreCallHook,
    Configs: ContainerConfigLookup,
{
    let caller = caller_defaults(request);
    let prepared = prepare_call(configs, &deps, request)?;
    let ready = match prepared {
        Prepared::Mock(response) => return Ok(response),
        Prepared::Call(ready) => ready,
    };
    // the awaited coroutine fails inside alist_container_files' try, whose except maps with the caller's custom_llm_provider
    let call = ready.handler_call::<Source>(request, deps.logging);
    // ast-grep-ignore: no-long-names
    async_container_file_list_handler(clients, hook, &call)
        .await
        .map_err(|error| {
            failure(
                ContainerFileListError::Handler(error),
                caller.provider,
                deps.redactor,
            )
        })
}
