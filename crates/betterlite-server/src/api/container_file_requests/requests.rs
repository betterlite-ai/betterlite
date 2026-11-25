use std::sync::Arc;

use axum::http::HeaderMap;
use axum::http::request::Parts;
use betterlite_core::api::container_ownership::ContainerForwardingParams;
use betterlite_core::api::request_form::convert::{
    FileData, FileDataValue, convert_uploads_to_file_data,
};
use betterlite_core::api::request_form::values::{FormParts, FormValue, get_form_data};
use betterlite_prompts::arguments::json::{Json, Object as JsonObject};

use super::ports::{ContainerAccess, RequestProcessing};
use super::{
    ContainerFileDownload, ContainerFileRequestError, ContainerFormData, ContainerFormValue,
    ContainerProcessingBody, ContainerProcessingReply, ContainerRequestData, FormReadError,
    ProviderHints, RequestFailure, content_type_for_file_id, pick_provider,
};
use crate::auth::auth_identity::AuthContext;

const FILE_ID_FIELD: &str = "file_id";
const CONTAINER_ID_FIELD: &str = "container_id";
const PROVIDER_FIELD: &str = "custom_llm_provider";
const MODEL_ID_FIELD: &str = "model_id";
const FILE_FIELD: &str = "file";
const CONTENT_ROUTE_TYPE: &str = "aretrieve_container_file_content";
const NON_BYTES_TYPE: &str = "json object";

// The upload route's per-route inputs: the catalog's async route name and B1101's decoded form
pub struct UploadRequest<'request> {
    pub route_type: &'request str,
    pub form: FormParts,
}

// The two processors the container file routes share; every collaborator is injected and the module keeps no state
#[derive(Clone)]
pub struct ContainerFileRequests {
    pub access: Arc<dyn ContainerAccess>,
    pub processing: Arc<dyn RequestProcessing>,
}

impl ContainerFileRequests {
    pub async fn process_file_content_request(
        &self,
        parts: &Parts,
        container_id: &str,
        file_id: &str,
        caller: &AuthContext,
    ) -> Result<ContainerFileDownload, ContainerFileRequestError> {
        let provider = pick_provider(&ProviderHints::from_request(parts));
        let identity = self
            .access
            .check_access(container_id, caller, &provider)
            .await?;
        let forwarding = self
            .access
            .forwarding_params(
                container_id,
                &identity.original_container_id,
                &identity.custom_llm_provider,
            )
            .await?;
        let data = ContainerRequestData::Json(content_data(file_id, forwarding));
        let (headers, bytes) = self.content_bytes(&data, caller).await?;
        Ok(ContainerFileDownload {
            bytes,
            content_type: content_type_for_file_id(file_id),
            headers,
        })
    }

    // Steps 5 to 8 of the content route: the processing call and the bytes check share one guarded section
    async fn content_bytes(
        &self,
        data: &ContainerRequestData,
        caller: &AuthContext,
    ) -> Result<(HeaderMap, Vec<u8>), ContainerFileRequestError> {
        let reply = match self
            .processing
            .process(data, CONTENT_ROUTE_TYPE, caller)
            .await
        {
            Ok(reply) => reply,
            Err(failure) => {
                return Err(self.fail(RequestFailure::from(failure), caller, data).await);
            }
        };
        let ContainerProcessingReply { headers, body } = reply;
        let ContainerProcessingBody::Bytes(bytes) = body else {
            let failure = RequestFailure::NotBytes {
                route_type: CONTENT_ROUTE_TYPE,
                got: NON_BYTES_TYPE,
            };
            return Err(self.fail(failure, caller, data).await);
        };
        Ok((headers, bytes))
    }

    pub async fn process_file_upload_request(
        &self,
        parts: &Parts,
        container_id: &str,
        caller: &AuthContext,
        upload: &UploadRequest<'_>,
    ) -> Result<ContainerProcessingReply, ContainerFileRequestError> {
        let route_type = upload.route_type;
        let fields = get_form_data(&upload.form).map_err(FormReadError::from)?;
        let converted = convert_uploads_to_file_data(&fields).map_err(FormReadError::from)?;
        let mut form_data = upload_data(converted);
        if !form_data.contains_key(FILE_FIELD) {
            return Err(ContainerFileRequestError::MissingFileField);
        }
        let first = match form_data.get(FILE_FIELD) {
            Some(ContainerFormValue::Tuples(files)) => files.first().cloned(),
            Some(ContainerFormValue::Field(_) | ContainerFormValue::Single(_)) | None => None,
        };
        if let Some(tuple) = first {
            form_data.insert(FILE_FIELD.to_owned(), ContainerFormValue::Single(tuple));
        }
        let provider = pick_provider(&ProviderHints::from_request(parts));
        let identity = self
            .access
            .check_access(container_id, caller, &provider)
            .await?;
        let forwarding = self
            .access
            .forwarding_params(
                container_id,
                &identity.original_container_id,
                &identity.custom_llm_provider,
            )
            .await?;
        push_forwarding(&mut form_data, forwarding);
        let data = ContainerRequestData::Form(form_data);
        match self.processing.process(&data, route_type, caller).await {
            Ok(reply) => Ok(reply),
            Err(failure) => Err(self
                .fail(RequestFailure::from(failure), caller, &data)
                .await),
        }
    }

    async fn fail(
        &self,
        failure: RequestFailure,
        caller: &AuthContext,
        data: &ContainerRequestData,
    ) -> ContainerFileRequestError {
        let mapped = self
            .processing
            .handle_exception(failure, caller, data)
            .await;
        ContainerFileRequestError::Processing(Box::new(mapped))
    }
}

// LiteLLM's data dict: file_id first, then the forwarding values in their own order, model_id only when one was found
fn content_data(file_id: &str, forwarding: ContainerForwardingParams) -> JsonObject {
    let mut data = JsonObject::new();
    data.insert(FILE_ID_FIELD.to_owned(), Json::Text(file_id.to_owned()));
    data.insert(
        CONTAINER_ID_FIELD.to_owned(),
        Json::Text(forwarding.container_id),
    );
    data.insert(
        PROVIDER_FIELD.to_owned(),
        Json::Text(forwarding.custom_llm_provider),
    );
    if let Some(model_id) = forwarding.model_id {
        data.insert(MODEL_ID_FIELD.to_owned(), Json::Text(model_id));
    }
    data
}

// convert_upload_files_to_file_data's answer in this module's value type
fn upload_data(converted: FileData) -> ContainerFormData {
    converted
        .into_iter()
        .map(|(name, value)| {
            let form_value = match value {
                FileDataValue::Unchanged(field) => ContainerFormValue::Field(field),
                FileDataValue::Files(files) => ContainerFormValue::Tuples(files),
            };
            (name, form_value)
        })
        .collect()
}

// data.update(get_container_forwarding_params(...)): the three keys replace the form's values and keep their slots
fn push_forwarding(data: &mut ContainerFormData, forwarding: ContainerForwardingParams) {
    data.insert(
        CONTAINER_ID_FIELD.to_owned(),
        ContainerFormValue::Field(FormValue::Text(forwarding.container_id)),
    );
    data.insert(
        PROVIDER_FIELD.to_owned(),
        ContainerFormValue::Field(FormValue::Text(forwarding.custom_llm_provider)),
    );
    if let Some(model_id) = forwarding.model_id {
        data.insert(
            MODEL_ID_FIELD.to_owned(),
            ContainerFormValue::Field(FormValue::Text(model_id)),
        );
    }
}
