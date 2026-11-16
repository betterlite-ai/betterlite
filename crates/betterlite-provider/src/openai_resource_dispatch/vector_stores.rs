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
use secrecy::SecretString;

pub struct Dispatch<'call> {
    pub settings: &'call Settings,
    pub globals: LiteLlmGlobals,
    pub env_api_key: Option<SecretString>,
    pub call: Call<'call>,
}

// B527's credential, URL, endpoint, request and reply calls, each calling its leaf
pub enum Call<'call> {
    Auth(Option<&'call SecretString>),
    Environment {
        headers: Headers,
        explicit_key: Option<SecretString>,
    },
    Url {
        api_base: Option<&'call str>,
        litellm_api_base: Option<&'call str>,
        vector_store_id: Option<&'call str>,
    },
    Endpoints(EndpointKind),
    Request(RequestCall<'call>),
    Reply {
        kind: ReplyKind,
        response: &'call http::Response<Vec<u8>>,
    },
}

#[derive(Clone, Copy)]
pub enum EndpointKind {
    Store,
    File,
}

pub enum RequestCall<'call> {
    Search {
        vector_store_id: &'call str,
        query: &'call Query,
        params: &'call SearchParams,
        api_base: &'call str,
    },
    Create {
        params: &'call CreateParams,
        api_base: &'call str,
    },
    FileCreate {
        params: &'call FileCreateParams,
        api_base: &'call str,
    },
    FileList {
        params: &'call FileListParams,
        api_base: &'call str,
    },
    File {
        operation: &'call FileOperation,
        file_id: &'call str,
        api_base: &'call str,
    },
}

#[derive(Debug)]
pub enum Outcome {
    Headers(Headers),
    Url(String),
    Endpoints(Box<VectorStoreIndexEndpoints>),
    Request(Request),
    Reply(VectorReply),
}

impl Dispatch<'_> {
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
            Call::Endpoints(kind) => Ok(Outcome::Endpoints(Box::new(endpoints(kind)))),
            Call::Request(request) => Ok(Outcome::Request(request_outcome(&request)?)),
            Call::Reply { kind, response } => {
                Ok(Outcome::Reply(transform_response(kind, response)?))
            }
        }
    }
}

fn endpoints(kind: EndpointKind) -> VectorStoreIndexEndpoints {
    match kind {
        EndpointKind::Store => store_endpoints(),
        EndpointKind::File => file_endpoints(),
    }
}

fn request_outcome(call: &RequestCall<'_>) -> Result<Request, VectorStoreError> {
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
