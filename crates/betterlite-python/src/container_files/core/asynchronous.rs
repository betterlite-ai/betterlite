use betterlite_core::api::container_file_endpoints::ContainerFileEndpoints;
use betterlite_core::api::container_file_list::alist_container_files;
use betterlite_core::api::container_file_upload::aupload_container_file;
use betterlite_openai::containers::container_file_http::GenericContainerHandler;

use super::{
    Answer, CONTENT, DELETE, LIST, RETRIEVE, SessionCore, UPLOAD, answer, encode, list_deps,
    upload_deps,
};
use crate::container_files::convert::{CallWire, ConvertError, query_values};
use crate::container_files::error::{SessionError, SessionFailure};
use crate::container_files::stack::{NoPreCall, Uploads};
use crate::container_files::transport::Source;

// the awaited twin of the synced dispatch: only the provider call under it differs
pub(super) async fn dispatch(
    core: &SessionCore,
    wire: &CallWire,
    blobs: &mut [Vec<u8>],
) -> Result<Answer, SessionFailure> {
    match wire.operation.as_str() {
        LIST => alist(core, wire).await,
        UPLOAD => aupload(core, wire, blobs).await,
        RETRIEVE => aendpoint(core, wire, RETRIEVE).await,
        DELETE => aendpoint(core, wire, DELETE).await,
        CONTENT => aendpoint(core, wire, CONTENT).await,
        other => Err(SessionFailure::from(ConvertError::Operation {
            operation: other.to_owned(),
        })),
    }
}

async fn alist(core: &SessionCore, wire: &CallWire) -> Result<Answer, SessionFailure> {
    let arguments = &wire.arguments;
    let extra_query = query_values(arguments.extra_query.as_ref())?;
    let request = arguments.list_request(extra_query.as_ref());
    let configs = core.configs(wire);
    let logging = core.logging_state(LIST, arguments.call_id()?.as_deref())?;
    let source = Source {
        client: core.client.clone(),
        deadline: core.deadline(arguments.timeout)?,
    };
    let deps = list_deps(&core.redactor, &logging, core.request_timeout_seconds);
    let response = alist_container_files(&source, &NoPreCall, &configs, deps, &request).await?;
    Ok(Answer::Json(encode(&response)?))
}

async fn aupload(
    core: &SessionCore,
    wire: &CallWire,
    blobs: &mut [Vec<u8>],
) -> Result<Answer, SessionFailure> {
    let arguments = &wire.arguments;
    let file = arguments.upload(blobs)?;
    let extra_query = query_values(arguments.extra_query.as_ref())?;
    let request = arguments.upload_request(&file, extra_query.as_ref());
    let configs = core.configs(wire);
    let logging = core.logging_state(UPLOAD, arguments.call_id()?.as_deref())?;
    let source = Source {
        client: core.client.clone(),
        deadline: core.deadline(arguments.timeout)?,
    };
    let uploads = Uploads {
        configs: &configs,
        source,
    };
    let deps = upload_deps(&core.redactor, &logging, core.request_timeout_seconds);
    let object = aupload_container_file(&uploads, deps, &request).await?;
    Ok(Answer::Json(encode(&object)?))
}

async fn aendpoint(
    core: &SessionCore,
    wire: &CallWire,
    name: &str,
) -> Result<Answer, SessionFailure> {
    let call = wire.arguments.endpoint_call()?;
    let configs = core.configs(wire);
    let logging = core.logging_state(name, call.litellm_call_id.as_deref())?;
    let source = Source {
        client: core.client.clone(),
        deadline: core.endpoint_deadline(call.timeout),
    };
    let handler: GenericContainerHandler<Source, NoPreCall> = GenericContainerHandler {
        clients: source,
        logging: NoPreCall,
    };
    let endpoints = ContainerFileEndpoints::new(&handler, &core.decoder, &configs, &core.redactor)
        .map_err(|source| SessionError::Catalog { source })?;
    let reply = endpoints
        .acontainer_file_call(name, &call, &logging, core.request_timeout_seconds)
        .await?;
    answer(reply)
}
