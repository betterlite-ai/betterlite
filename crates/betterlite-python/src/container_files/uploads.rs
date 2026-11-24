use std::collections::BTreeMap;

use betterlite_core::api::container_file_endpoints::handler::ContainerConfigLookup;
use betterlite_core::api::container_file_upload::execution::{
    AsyncUploadExecutor, ContainerFileUploadCall, UploadExecutor,
};
use betterlite_core::containers::container_request_parameters::ContainerProviderConfig;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::container_contracts::config::BaseContainerConfig;
use betterlite_openai::api::container_contracts::error::BaseLlmException;
use betterlite_openai::api::container_contracts::records::ContainerFileObject;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::containers::container_file_http::{
    ContainerFileHttpCall, ContainerFileHttpError, ContainerFileReply, GenericContainerHandler,
};
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;

use super::stack::{Configs, NoPreCall};
use super::transport::Source;

const CONTAINER_ID: &str = "container_id";

// B3367's executor: one upload through B3363's handler, with the config the caller's provider names
pub(crate) struct Uploads<'upload> {
    pub(crate) configs: &'upload Configs,
    pub(crate) source: Source,
}

struct ReadyUpload<'configs> {
    config: &'configs dyn BaseContainerConfig,
    params: Object,
    values: BTreeMap<String, ParamValue>,
}

impl ReadyUpload<'_> {
    fn http_call<'call>(
        &'call self,
        call: &'call ContainerFileUploadCall<'call>,
    ) -> ContainerFileHttpCall<'call, Source> {
        ContainerFileHttpCall {
            endpoint_name: call.endpoint_name,
            config: self.config,
            litellm_params: &self.params,
            logging: call.logging,
            extra_headers: call.extra_headers,
            extra_query: call.extra_query,
            timeout: None,
            values: &self.values,
            file: Some(call.file),
            blocking_client: None,
            async_client: None,
        }
    }
}

impl<'upload> Uploads<'upload> {
    fn ready(
        &self,
        call: &ContainerFileUploadCall<'_>,
    ) -> Result<ReadyUpload<'upload>, ContainerFileHttpError> {
        let config = self.config(call.provider_config)?;
        let params = params_object(call.litellm_params)?;
        let mut values = BTreeMap::new();
        values.insert(
            CONTAINER_ID.to_owned(),
            ParamValue::Text(call.container_id.to_owned()),
        );
        Ok(ReadyUpload {
            config,
            params,
            values,
        })
    }

    fn config(
        &self,
        kind: ContainerProviderConfig,
    ) -> Result<&'upload dyn BaseContainerConfig, ContainerFileHttpError> {
        self.configs
            .container_config(provider_id(kind))
            .ok_or_else(|| missing_config(kind))
    }
}

impl UploadExecutor for Uploads<'_> {
    type Failure = ContainerFileHttpError;

    fn upload(
        &self,
        call: &ContainerFileUploadCall<'_>,
    ) -> Result<ContainerFileObject, Self::Failure> {
        let ready = self.ready(call)?;
        let handler = GenericContainerHandler {
            clients: self.source.clone(),
            logging: NoPreCall,
        };
        file_object(handler.handle(&ready.http_call(call))?)
    }
}

impl AsyncUploadExecutor for Uploads<'_> {
    type Failure = ContainerFileHttpError;

    async fn aupload(
        &self,
        call: &ContainerFileUploadCall<'_>,
    ) -> Result<ContainerFileObject, Self::Failure> {
        let ready = self.ready(call)?;
        let handler = GenericContainerHandler {
            clients: self.source.clone(),
            logging: NoPreCall,
        };
        file_object(handler.ahandle(&ready.http_call(call)).await?)
    }
}

const fn provider_id(kind: ContainerProviderConfig) -> ProviderId {
    match kind {
        ContainerProviderConfig::OpenAi => ProviderId::Openai,
        ContainerProviderConfig::Azure => ProviderId::Azure,
    }
}

const fn provider_name(kind: ContainerProviderConfig) -> &'static str {
    match kind {
        ContainerProviderConfig::OpenAi => "openai",
        ContainerProviderConfig::Azure => "azure",
    }
}

fn missing_config(kind: ContainerProviderConfig) -> ContainerFileHttpError {
    ContainerFileHttpError::Provider(BaseLlmException {
        status_code: 400,
        message: format!(
            "Container provider config not found for provider: {}",
            provider_name(kind)
        ),
        headers: BTreeMap::new(),
    })
}

// B3367 hands the config the kwargs `GenericLiteLLMParams(**kwargs)` became, as the JSON object B3363 reads
fn params_object(params: &IndexMap<String, JsonValue>) -> Result<Object, ContainerFileHttpError> {
    let text =
        serde_json::to_string(params).map_err(|source| ContainerFileHttpError::Json { source })?;
    serde_json::from_str(&text).map_err(|source| ContainerFileHttpError::Json { source })
}

fn file_object(reply: ContainerFileReply) -> Result<ContainerFileObject, ContainerFileHttpError> {
    let ContainerFileReply::File(object) = reply else {
        return Err(ContainerFileHttpError::Provider(BaseLlmException {
            status_code: 500,
            message: String::from("upload_container_file did not answer a container file"),
            headers: BTreeMap::new(),
        }));
    };
    Ok(object)
}
