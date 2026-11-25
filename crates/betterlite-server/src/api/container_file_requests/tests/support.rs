use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Request};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::api::container_ownership::errors::ContainerOwnershipError;
use betterlite_core::api::container_ownership::repository::{
    ContainerObjectTable, ContainerOwnerRow, ContainerOwnerUpdate, ContainerOwnerWrite,
};
use betterlite_core::api::container_ownership::{ContainerForwardingParams, ContainerIdentity};
use betterlite_core::api::request_form::values::{
    FormPart, FormParts, FormValue, PartValue, UploadFile,
};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_prompts::arguments::json::{Json, Object as JsonObject};

use crate::api::container_file_requests::ports::{ContainerAccess, RequestProcessing};
use crate::api::container_file_requests::requests::{ContainerFileRequests, UploadRequest};
use crate::api::container_file_requests::{
    ContainerFormValue, ContainerProcessingBody, ContainerProcessingReply, ContainerRequestData,
    RequestFailure,
};
use crate::auth::auth_identity::AuthContext;
use crate::pre_call::failure::{PreCallFailure, ProxyFailure};

pub(super) fn caller(subject: &str) -> AuthContext {
    let document = format!(
        r#"{{"principal_type":"human","subject":"{subject}","auth_method":"api_key","user":{{"id":"{subject}"}}}}"#
    );
    let principal: Principal = serde_json::from_str(&document).unwrap();
    AuthContext { principal }
}

pub(super) fn request_parts(uri: &str, provider: Option<&str>) -> Parts {
    let mut builder = Request::builder().method("POST").uri(uri);
    if let Some(header) = provider {
        builder = builder.header("custom-llm-provider", header);
    }
    let request = builder.body(()).unwrap();
    request.into_parts().0
}

pub(super) fn text_part(name: &str, value: &str) -> FormPart {
    FormPart {
        name: name.to_owned(),
        value: PartValue::Text(value.to_owned()),
    }
}

pub(super) fn upload_part(name: &str, filename: &str) -> FormPart {
    FormPart {
        name: name.to_owned(),
        value: PartValue::Upload(UploadFile {
            filename: filename.to_owned(),
            content_type: Some("text/csv".to_owned()),
            content: filename.as_bytes().to_vec(),
            size: None,
        }),
    }
}

pub(super) fn form(items: Vec<FormPart>) -> FormParts {
    FormParts { items }
}

pub(super) struct FakeAccess {
    pub(super) identity: ContainerIdentity,
    pub(super) forwarding: ContainerForwardingParams,
    pub(super) deny: bool,
    pub(super) refuse_forwarding: bool,
    events: Mutex<Vec<String>>,
}

impl FakeAccess {
    pub(super) fn granting() -> Self {
        Self {
            identity: ContainerIdentity {
                original_container_id: "cntr-upstream".to_owned(),
                custom_llm_provider: "openai".to_owned(),
            },
            forwarding: ContainerForwardingParams {
                container_id: "cntr-upstream".to_owned(),
                custom_llm_provider: "openai".to_owned(),
                model_id: None,
            },
            deny: false,
            refuse_forwarding: false,
            events: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

#[async_trait]
impl ContainerAccess for FakeAccess {
    async fn check_access(
        &self,
        container_id: &str,
        caller: &AuthContext,
        provider: &str,
    ) -> Result<ContainerIdentity, ContainerOwnershipError> {
        let subject = caller.principal.subject.as_str();
        let event = format!("check:{container_id}:{subject}:{provider}");
        self.events.lock().unwrap().push(event);
        if self.deny {
            return Err(ContainerOwnershipError::Forbidden);
        }
        Ok(self.identity.clone())
    }

    async fn forwarding_params(
        &self,
        container_id: &str,
        original_container_id: &str,
        provider: &str,
    ) -> Result<ContainerForwardingParams, ContainerOwnershipError> {
        let event = ["forward", container_id, original_container_id, provider].join(":");
        self.events.lock().unwrap().push(event);
        let forwarding = self.forwarding.clone();
        let refused = self.refuse_forwarding;
        if refused {
            return Err(ContainerOwnershipError::NoIdentityScope);
        }
        Ok(forwarding)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RecordedCall {
    pub(super) data: ContainerRequestData,
    pub(super) route_type: String,
    pub(super) model: Option<String>,
}

#[derive(Default)]
pub(super) struct FakeProcessing {
    reply: Mutex<Option<Result<ContainerProcessingReply, PreCallFailure>>>,
    pub(super) mapped: Mutex<Option<ProxyException>>,
    processed: Mutex<Vec<RecordedCall>>,
    failures: Mutex<Vec<RequestFailure>>,
}

impl FakeProcessing {
    pub(super) fn replying(reply: ContainerProcessingReply) -> Self {
        let fake = Self::default();
        *fake.reply.lock().unwrap() = Some(Ok(reply));
        fake
    }

    pub(super) fn failing(failure: PreCallFailure) -> Self {
        let fake = Self::default();
        *fake.reply.lock().unwrap() = Some(Err(failure));
        fake
    }

    pub(super) fn processed(&self) -> Vec<RecordedCall> {
        self.processed.lock().unwrap().clone()
    }

    pub(super) fn failures(&self) -> Vec<RequestFailure> {
        self.failures.lock().unwrap().clone()
    }
}

#[async_trait]
impl RequestProcessing for FakeProcessing {
    async fn process(
        &self,
        data: &ContainerRequestData,
        route_type: &str,
        _caller: &AuthContext,
    ) -> Result<ContainerProcessingReply, PreCallFailure> {
        self.processed.lock().unwrap().push(RecordedCall {
            data: data.clone(),
            route_type: route_type.to_owned(),
            model: data_model(data),
        });
        let pending = self.reply.lock().unwrap().take();
        pending.unwrap_or_else(|| panic!("the fake has no reply"))
    }

    async fn handle_exception(
        &self,
        failure: RequestFailure,
        _caller: &AuthContext,
        _data: &ContainerRequestData,
    ) -> ProxyException {
        self.failures.lock().unwrap().push(failure);
        let mapped = self.mapped.lock().unwrap().take();
        mapped.unwrap_or_else(mapped_exception)
    }
}

fn data_model(data: &ContainerRequestData) -> Option<String> {
    match data {
        ContainerRequestData::Json(object) => match object.get("model") {
            Some(Json::Text(model)) => Some(model.clone()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        },
        ContainerRequestData::Form(fields) => match fields.get("model") {
            Some(ContainerFormValue::Field(FormValue::Text(model))) => Some(model.clone()),
            Some(
                ContainerFormValue::Field(
                    FormValue::Upload(_)
                    | FormValue::List(_)
                    | FormValue::Number(_)
                    | FormValue::Other(_)
                    | FormValue::NonFinite(_),
                )
                | ContainerFormValue::Tuples(_)
                | ContainerFormValue::Single(_),
            )
            | None => None,
        },
    }
}

pub(super) fn mapped_exception() -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: "mapped failure".to_owned(),
        error_type: "internal_server_error".to_owned(),
        param: None,
        code: ExceptionCode::Status(500),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

pub(super) fn proxy_failure() -> PreCallFailure {
    PreCallFailure::Proxy(ProxyFailure {
        message: "the router refused".to_owned(),
        error_type: "internal_server_error".to_owned(),
        param: None,
        code: 500,
    })
}

pub(super) fn bytes_reply(bytes: &[u8]) -> ContainerProcessingReply {
    ContainerProcessingReply {
        headers: call_id_headers(),
        body: ContainerProcessingBody::Bytes(bytes.to_vec()),
    }
}

pub(super) fn json_reply() -> ContainerProcessingReply {
    ContainerProcessingReply {
        headers: call_id_headers(),
        body: ContainerProcessingBody::Json(JsonObject::new()),
    }
}

fn call_id_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-litellm-call-id", HeaderValue::from_static("call-1"));
    headers
}

pub(super) fn container_requests(
    access: Arc<FakeAccess>,
    processing: Arc<FakeProcessing>,
) -> ContainerFileRequests {
    ContainerFileRequests { access, processing }
}

pub(super) fn file_upload_form() -> FormParts {
    form(vec![
        text_part("purpose", "assistants"),
        upload_part("file", "data.csv"),
    ])
}

pub(super) fn upload_request(route_type: &str, form: FormParts) -> UploadRequest<'_> {
    UploadRequest { route_type, form }
}

#[derive(Default)]
pub(super) struct FakeObjects {
    pub(super) rows: Mutex<BTreeMap<String, ContainerOwnerRow>>,
}

impl ContainerObjectTable for FakeObjects {
    fn availability(&self) -> Result<(), TableError> {
        Ok(())
    }

    fn find_row(
        &self,
        model_object_id: &str,
        _file_purpose: Option<&str>,
    ) -> impl Future<Output = Result<Option<ContainerOwnerRow>, TableError>> + Send {
        let row = self.rows.lock().unwrap().get(model_object_id).cloned();
        ready(Ok(row))
    }

    fn find_owned(
        &self,
        _owner_scopes: Vec<String>,
    ) -> impl Future<Output = Result<Vec<String>, TableError>> + Send {
        ready(Ok(Vec::new()))
    }

    fn create(
        &self,
        _write: ContainerOwnerWrite,
    ) -> impl Future<Output = Result<(), TableError>> + Send {
        ready(Ok(()))
    }

    fn update(
        &self,
        _update: ContainerOwnerUpdate,
    ) -> impl Future<Output = Result<(), TableError>> + Send {
        ready(Ok(()))
    }
}
