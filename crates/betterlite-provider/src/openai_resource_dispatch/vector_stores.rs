use betterlite_config::sections::openai_credentials::Settings;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreIndexEndpoints;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::vector_stores::error::VectorStoreError;
use betterlite_openai::vector_stores::files::{
    FileCreateParams, FileListParams, FileOperation, create_request as file_create_request,
    file_request, list_request as file_list_request,
};
use betterlite_openai::vector_stores::reply::{ReplyKind, VectorReply, transform_response};
use betterlite_openai::vector_stores::store::{
    CreateParams, Query, SearchParams, create_request as store_create_request, search_request,
};
use betterlite_openai::vector_stores::{
    Headers, Request, complete_url, file_endpoints, get_auth_credentials, store_endpoints,
    validate_environment,
};
use http::Response;
use secrecy::SecretString;

pub struct Dispatch<'a> {
    pub settings: &'a Settings,
    pub globals: LiteLlmGlobals,
    pub env_api_key: Option<SecretString>,
    pub call: Call<'a>,
}

// B527's credential, URL, endpoint, search, create and attached-file calls, each calling its leaf
pub enum Call<'a> {
    Auth(Option<&'a SecretString>),
    Environment {
        headers: Headers,
        explicit_key: Option<SecretString>,
    },
    Url {
        api_base: Option<&'a str>,
        litellm_api_base: Option<&'a str>,
        vector_store_id: Option<&'a str>,
    },
    Endpoints(EndpointKind),
    Request(RequestCall<'a>),
    Reply(ReplyCall<'a>),
}

#[derive(Clone, Copy)]
pub enum EndpointKind {
    Store,
    File,
}

pub enum RequestCall<'a> {
    Search {
        vector_store_id: &'a str,
        query: &'a Query,
        params: &'a SearchParams,
        api_base: &'a str,
    },
    Create {
        params: &'a CreateParams,
        api_base: &'a str,
    },
    FileCreate {
        params: &'a FileCreateParams,
        api_base: &'a str,
    },
    FileList {
        params: &'a FileListParams,
        api_base: &'a str,
    },
    File {
        operation: &'a FileOperation,
        file_id: &'a str,
        api_base: &'a str,
    },
}

pub enum ReplyCall<'a> {
    Search(&'a Response<Vec<u8>>),
    Create(&'a Response<Vec<u8>>),
    File(&'a Response<Vec<u8>>),
}

#[derive(Debug)]
pub enum Outcome {
    Headers(Headers),
    Url(String),
    Endpoints(Box<VectorStoreIndexEndpoints>),
    Request(Request),
    Reply(Box<VectorReply>),
}

impl Dispatch<'_> {
    #[must_use]
    pub fn run(self) -> Result<Outcome, VectorStoreError> {
        let Self {
            settings,
            globals,
            env_api_key,
            call,
        } = self;
        match call {
            Call::Auth(api_key) => Ok(Outcome::Headers(get_auth_credentials(api_key)?)),
            Call::Environment {
                headers,
                explicit_key,
            } => Ok(Outcome::Headers(validate_environment(
                headers,
                explicit_key,
                globals,
                env_api_key,
            ))),
            Call::Url {
                api_base,
                litellm_api_base,
                vector_store_id,
            } => Ok(Outcome::Url(complete_url(
                api_base,
                litellm_api_base,
                settings,
                vector_store_id,
            )?)),
            Call::Endpoints(EndpointKind::Store) => {
                Ok(Outcome::Endpoints(Box::new(store_endpoints())))
            }
            Call::Endpoints(EndpointKind::File) => {
                Ok(Outcome::Endpoints(Box::new(file_endpoints())))
            }
            Call::Request(request) => Ok(Outcome::Request(request_outcome(request)?)),
            Call::Reply(ReplyCall::Search(response)) => Ok(Outcome::Reply(Box::new(
                transform_response(ReplyKind::Search, response)?,
            ))),
            Call::Reply(ReplyCall::Create(response)) => Ok(Outcome::Reply(Box::new(
                transform_response(ReplyKind::Create, response)?,
            ))),
            Call::Reply(ReplyCall::File(response)) => Ok(Outcome::Reply(Box::new(
                transform_response(ReplyKind::File, response)?,
            ))),
        }
    }
}

fn request_outcome(call: RequestCall<'_>) -> Result<Request, VectorStoreError> {
    Ok(match call {
        RequestCall::Search {
            vector_store_id,
            query,
            params,
            api_base,
        } => search_request(vector_store_id, query, params, api_base)?,
        RequestCall::Create { params, api_base } => store_create_request(params, api_base),
        RequestCall::FileCreate { params, api_base } => file_create_request(params, api_base),
        RequestCall::FileList { params, api_base } => file_list_request(params, api_base),
        RequestCall::File {
            operation,
            file_id,
            api_base,
        } => file_request(operation, file_id, api_base)?,
    })
}
