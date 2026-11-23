use betterlite_core::api::container_create_list::{acreate_container, alist_containers};

use crate::container_calls::convert::{CallWire, ConvertError, query_values, requested_timeout};
use crate::container_calls::error::SessionFailure;

use super::{
    CREATE, CollectionCall, DELETE, LIST, RETRIEVE, SessionCore, encode, encode_created,
    resource_call_type,
};

// the async twin of the blocking call: the same steps through the async handler and the operations' async forms
pub(crate) async fn acall(core: &SessionCore, wire: &CallWire) -> Result<String, SessionFailure> {
    match wire.operation.as_str() {
        CREATE => {
            acollection(
                core,
                wire,
                CollectionCall::Create(&wire.arguments.create()?),
            )
            .await
        }
        LIST => acollection(core, wire, CollectionCall::List(&wire.arguments.list()?)).await,
        RETRIEVE => aresource(core, wire, false).await,
        DELETE => aresource(core, wire, true).await,
        other => Err(SessionFailure::from(ConvertError::Operation {
            operation: other.to_owned(),
        })),
    }
}

// LiteLLM's acreate_container and alist_containers, through the async handler
async fn acollection(
    core: &SessionCore,
    wire: &CallWire,
    call: CollectionCall<'_>,
) -> Result<String, SessionFailure> {
    let (configs, logging, handler) = core.collection_pieces(wire, &call)?;
    let deps = core.deps(&logging);
    match call {
        CollectionCall::Create(create) => {
            let created = acreate_container(&handler, &configs, deps, create).await?;
            encode_created(&created)
        }
        CollectionCall::List(list) => {
            encode(&alist_containers(&handler, &configs, deps, list).await?)
        }
    }
}

// LiteLLM's aretrieve_container and adelete_container: the async handler keeps the provider's own id
async fn aresource(
    core: &SessionCore,
    wire: &CallWire,
    delete: bool,
) -> Result<String, SessionFailure> {
    let extra_query = query_values(wire.arguments.extra_query.as_ref())?;
    let timeout = requested_timeout(wire.arguments.timeout)?;
    let request = wire
        .arguments
        .operation_request(extra_query.as_ref(), timeout);
    let (configs, logging, handler) =
        core.operation_stack(wire, &request, resource_call_type(delete))?;
    let operations = core.operations(&handler, &configs, &logging);
    if delete {
        encode(&operations.adelete_container(&request).await?)
    } else {
        encode(&operations.aretrieve_container(&request).await?)
    }
}
