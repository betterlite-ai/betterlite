use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};

use crate::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery, ContainerRequestContext,
};
use crate::api::container_contracts::error::{BaseLlmException, ContainerError};
use crate::api::container_contracts::records::{
    ContainerFileListResponse, ContainerListResponse, ContainerObject, DeleteContainerResult,
};
use crate::containers::costs::ContainerCosts;
use crate::containers::urls::ContainerResource;
use crate::credentials::{
    ExplicitCredentials, LiteLlmGlobals, get_openai_credentials, resolve_openai_api_base,
};

pub mod costs;
pub mod replies;
pub mod requests;
pub mod urls;

mod tests;

#[derive(Debug)]
pub struct OpenAiContainerConfig {
    pub settings: OpenAiCredentialsSettings,
    pub globals: LiteLlmGlobals,
    pub env_api_key: Option<SecretString>,
    pub costs: Arc<dyn ContainerCosts>,
}

impl BaseContainerConfig for OpenAiContainerConfig {
    fn get_supported_openai_params(&self) -> Vec<String> {
        requests::SUPPORTED_PARAMS
            .iter()
            .copied()
            .map(str::to_owned)
            .collect()
    }

    fn map_openai_params(
        &self,
        // ast-grep-ignore: no-long-names
        container_create_optional_params: &Map<String, Value>,
        _drop_params: bool,
    ) -> Map<String, Value> {
        // ast-grep-ignore: no-long-names
        container_create_optional_params.clone()
    }

    fn validate_environment(
        &self,
        headers: &BTreeMap<String, String>,
        api_key: Option<&str>,
    ) -> BTreeMap<String, String> {
        let explicit = ExplicitCredentials {
            api_base: None,
            api_key: api_key.map(SecretString::from),
            organization: None,
        };
        let globals = LiteLlmGlobals {
            api_base: self.globals.api_base.clone(),
            api_key: self.globals.api_key.clone(),
            openai_key: self.globals.openai_key.clone(),
            organization: self.globals.organization.clone(),
        };
        let resolved =
            get_openai_credentials(explicit, globals, &self.settings, self.env_api_key.clone());
        let authorization = resolved.api_key.as_ref().map_or_else(
            || String::from("Bearer None"),
            |secret| format!("Bearer {}", secret.expose_secret()),
        );
        let mut merged = headers.clone();
        merged.insert(String::from("Authorization"), authorization);
        merged
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        _litellm_params: &Map<String, Value>,
    ) -> Result<String, ContainerError> {
        let base =
            resolve_openai_api_base(api_base, self.globals.api_base.as_deref(), &self.settings);
        let trimmed = base.trim_end_matches('/');
        Ok(format!("{trimmed}/containers"))
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_create_request(
        &self,
        name: &str,
        // ast-grep-ignore: no-long-names
        container_create_optional_request_params: &Map<String, Value>,
        _context: ContainerRequestContext<'_>,
    ) -> Map<String, Value> {
        // ast-grep-ignore: no-long-names
        requests::create_request(name, container_create_optional_request_params)
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_create_response(
        &self,
        raw_response: &[u8],
    ) -> Result<ContainerObject, ContainerError> {
        let mut container: ContainerObject = replies::decode_container_reply(raw_response)?;
        let cost = self.costs.code_interpreter_cost(1, "openai");
        container.hidden_params = costs::cost_hidden_params(cost)?;
        Ok(container)
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_list_request(
        &self,
        api_base: &str,
        _context: ContainerRequestContext<'_>,
        query: &ContainerListQuery<'_>,
    ) -> (String, Map<String, Value>) {
        (api_base.to_owned(), requests::list_query(query))
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_list_response(
        &self,
        raw_response: &[u8],
    ) -> Result<ContainerListResponse, ContainerError> {
        replies::decode_container_reply(raw_response)
    }

    #[rustfmt::skip]
    // ast-grep-ignore: no-long-names
    fn transform_container_retrieve_request(
        &self, container_id: &str, api_base: &str, _context: ContainerRequestContext<'_>,
    ) -> Result<(String, Map<String, Value>), ContainerError> {
        let url = urls::resource_url(api_base, ContainerResource::Container(container_id))?;
        Ok((url, Map::new()))
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_retrieve_response(
        &self,
        raw_response: &[u8],
    ) -> Result<ContainerObject, ContainerError> {
        replies::decode_container_reply(raw_response)
    }

    // LiteLLM builds the delete request exactly as the retrieve request
    #[rustfmt::skip]
    // ast-grep-ignore: no-long-names
    fn transform_container_delete_request(
        &self, container_id: &str, api_base: &str, context: ContainerRequestContext<'_>,
    ) -> Result<(String, Map<String, Value>), ContainerError> {
        self.transform_container_retrieve_request(container_id, api_base, context)
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_delete_response(
        &self,
        raw_response: &[u8],
    ) -> Result<DeleteContainerResult, ContainerError> {
        replies::decode_container_reply(raw_response)
    }

    #[rustfmt::skip]
    // ast-grep-ignore: no-long-names
    fn transform_container_file_list_request(
        &self, container_id: &str, api_base: &str, _context: ContainerRequestContext<'_>, query: &ContainerListQuery<'_>,
    ) -> Result<(String, Map<String, Value>), ContainerError> {
        let url = urls::resource_url(api_base, ContainerResource::Files(container_id))?;
        Ok((url, requests::list_query(query)))
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_file_list_response(
        &self,
        raw_response: &[u8],
    ) -> Result<ContainerFileListResponse, ContainerError> {
        replies::decode_container_reply(raw_response)
    }

    #[rustfmt::skip]
    // ast-grep-ignore: no-long-names
    fn transform_container_file_content_request(
        &self, container_id: &str, file_id: &str, api_base: &str, _context: ContainerRequestContext<'_>,
    ) -> Result<(String, Map<String, Value>), ContainerError> {
        let resource = ContainerResource::FileContent { container_id, file_id };
        Ok((urls::resource_url(api_base, resource)?, Map::new()))
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_file_content_response(
        &self,
        raw_response: &[u8],
    ) -> Result<Vec<u8>, ContainerError> {
        Ok(raw_response.to_vec())
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &BTreeMap<String, String>,
    ) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: error_message.to_owned(),
            headers: headers.clone(),
        }
    }
}
