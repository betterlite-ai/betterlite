use std::sync::Arc;

use crate::assistants::client::{ClientKind, ClientOptions, SdkClient};
use crate::assistants::error::AssistantsError;
use crate::assistants::records::{
    AssistantCreate, AssistantDeleted, AssistantObject, AssistantPage, ListParams,
};
use crate::assistants::transport::AssistantsTransport;

pub struct AssistantsApi<T> {
    pub transport: Arc<T>,
}

impl<T: AssistantsTransport> AssistantsApi<T> {
    // Serves get_openai_client (Sync) and async_get_openai_client (Async): a supplied client wins untouched.
    #[must_use]
    pub fn get_openai_client(
        &self,
        kind: ClientKind,
        options: ClientOptions,
        client: Option<SdkClient<T>>,
    ) -> SdkClient<T> {
        match client {
            Some(supplied) => supplied,
            None => SdkClient {
                kind,
                api_key: options.api_key,
                base_url: options.api_base,
                timeout: options.timeout,
                max_retries: options.max_retries,
                organization: options.organization,
                transport: Arc::clone(&self.transport),
            },
        }
    }

    pub async fn async_get_assistants(
        &self,
        options: ClientOptions,
        client: Option<SdkClient<T>>,
        params: &ListParams,
    ) -> Result<AssistantPage, AssistantsError> {
        self.get_openai_client(ClientKind::Async, options, client)
            .list(params.request_params())
            .await
    }

    // Only the literal true selects the async helper, and that helper gets its own defaults rather than the caller's pagination.
    pub async fn get_assistants(
        &self,
        options: ClientOptions,
        client: Option<SdkClient<T>>,
        aget_assistants: Option<bool>,
        params: &ListParams,
    ) -> Result<AssistantPage, AssistantsError> {
        let kind = client_kind(aget_assistants);
        let request_params = match kind {
            ClientKind::Async => ListParams::default().request_params(),
            ClientKind::Sync => params.request_params(),
        };
        let listing = self.get_openai_client(kind, options, client);
        listing.list(request_params).await
    }

    // Serves create_assistants and async_create_assistants: the flag only changes the client type.
    pub async fn create_assistants(
        &self,
        options: ClientOptions,
        create_assistant_data: &AssistantCreate,
        client: Option<SdkClient<T>>,
        async_create_assistants: Option<bool>,
    ) -> Result<AssistantObject, AssistantsError> {
        let kind = client_kind(async_create_assistants);
        let creator = self.get_openai_client(kind, options, client);
        creator.create(create_assistant_data).await
    }

    // Serves delete_assistant and async_delete_assistant: the flag only changes the client type.
    pub async fn delete_assistant(
        &self,
        options: ClientOptions,
        assistant_id: &str,
        client: Option<SdkClient<T>>,
        async_delete_assistants: Option<bool>,
    ) -> Result<AssistantDeleted, AssistantsError> {
        let kind = client_kind(async_delete_assistants);
        self.get_openai_client(kind, options, client)
            .delete(assistant_id)
            .await
    }
}

const fn client_kind(flag: Option<bool>) -> ClientKind {
    if matches!(flag, Some(true)) {
        ClientKind::Async
    } else {
        ClientKind::Sync
    }
}
