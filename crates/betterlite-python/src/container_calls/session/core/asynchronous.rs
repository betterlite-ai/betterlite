use betterlite_core::api::container_create_list::handler::GenericCollectionHandler;
use betterlite_core::api::container_create_list::{
    ContainerCallDeps, ContainerCreateCall, ContainerListCall, acreate_container, alist_containers,
};
use betterlite_core::api::container_resource_ops::ContainerOperationRequest;

use crate::container_calls::convert::CallWire;
use crate::container_calls::error::SessionFailure;
use crate::container_calls::stack;
use crate::container_calls::transport::Source;

use super::{
    CREATE_CALL_TYPE, DELETE_CALL_TYPE, LIST_CALL_TYPE, RETRIEVE_CALL_TYPE, SessionCore, encode,
    encode_created,
};

// the async twins of the four calls: the same steps through core's async handler and operations
impl SessionCore {
    // LiteLLM's acreate_container: the same steps through the async handler
    pub(crate) async fn acreate(
        &self,
        wire: &CallWire,
        call: &ContainerCreateCall,
    ) -> Result<String, SessionFailure> {
        let configs = self.configs(wire);
        let logging = stack::logging_state(
            CREATE_CALL_TYPE,
            call.litellm_call_id.as_deref(),
            &self.redactor,
        )?;
        let source = Source {
            client: self.client.clone(),
            deadline: self.deadline(call.timeout)?,
        };
        let handler = GenericCollectionHandler {
            source,
            hook: stack::NoPreCall,
        };
        let deps = ContainerCallDeps {
            logging: &logging,
            redactor: &self.redactor,
            request_timeout_seconds: self.request_timeout_seconds,
        };
        let created = acreate_container(&handler, &configs, deps, call).await?;
        encode_created(&created)
    }

    // LiteLLM's alist_containers: the same steps through the async handler
    pub(crate) async fn alist(
        &self,
        wire: &CallWire,
        call: &ContainerListCall,
    ) -> Result<String, SessionFailure> {
        let configs = self.configs(wire);
        let logging = stack::logging_state(
            LIST_CALL_TYPE,
            call.litellm_call_id.as_deref(),
            &self.redactor,
        )?;
        let source = Source {
            client: self.client.clone(),
            deadline: self.deadline(call.timeout)?,
        };
        let handler = GenericCollectionHandler {
            source,
            hook: stack::NoPreCall,
        };
        let deps = ContainerCallDeps {
            logging: &logging,
            redactor: &self.redactor,
            request_timeout_seconds: self.request_timeout_seconds,
        };
        let reply = alist_containers(&handler, &configs, deps, call).await?;
        encode(&reply)
    }

    // LiteLLM's aretrieve_container: the async handler keeps the provider's own id
    pub(crate) async fn aretrieve(
        &self,
        wire: &CallWire,
        request: &ContainerOperationRequest<'_>,
    ) -> Result<String, SessionFailure> {
        let (configs, logging, handler) =
            self.operation_stack(wire, request, RETRIEVE_CALL_TYPE)?;
        let operations = self.operations(&handler, &configs, &logging);
        let reply = operations.aretrieve_container(request).await?;
        encode(&reply)
    }

    pub(crate) async fn adelete(
        &self,
        wire: &CallWire,
        request: &ContainerOperationRequest<'_>,
    ) -> Result<String, SessionFailure> {
        let (configs, logging, handler) = self.operation_stack(wire, request, DELETE_CALL_TYPE)?;
        let operations = self.operations(&handler, &configs, &logging);
        let reply = operations.adelete_container(request).await?;
        encode(&reply)
    }
}
