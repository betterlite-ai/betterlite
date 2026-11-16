use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::api::container_contracts::error::ContainerError;
use betterlite_openai::api::container_contracts::records::{
    ContainerFileListResponse, ContainerListResponse, ContainerObject, DeleteContainerResult,
};
use betterlite_openai::openai_base_config::Object;

pub struct Dispatch<'a> {
    pub config: &'a dyn BaseContainerConfig,
    pub context: ContainerRequestContext<'a>,
    pub call: Call<'a>,
}

// B522's six resource operations; a request transform and its reply decode stay separate calls
pub enum Call<'a> {
    Request(RequestCall<'a>),
    Reply(ReplyCall<'a>),
}

pub enum RequestCall<'a> {
    Create {
        name: &'a str,
        params: &'a Object,
    },
    List {
        api_base: &'a str,
        query: ContainerListQuery<'a>,
    },
    Retrieve {
        container_id: &'a str,
        api_base: &'a str,
    },
    Delete {
        container_id: &'a str,
        api_base: &'a str,
    },
    FileList {
        container_id: &'a str,
        api_base: &'a str,
        query: ContainerListQuery<'a>,
    },
    FileContent {
        container_id: &'a str,
        file_id: &'a str,
        api_base: &'a str,
    },
}

pub enum ReplyCall<'a> {
    Create(&'a [u8]),
    List(&'a [u8]),
    Retrieve(&'a [u8]),
    Delete(&'a [u8]),
    FileList(&'a [u8]),
    FileContent(&'a [u8]),
}

#[derive(Debug)]
pub enum Outcome {
    Create(Object),
    Request { url: String, query: Object },
    Container(Box<ContainerObject>),
    List(Box<ContainerListResponse>),
    Deleted(Box<DeleteContainerResult>),
    FileList(Box<ContainerFileListResponse>),
    FileContent(Box<[u8]>),
}

impl Dispatch<'_> {
    #[must_use]
    pub fn run(self) -> Result<Outcome, ContainerError> {
        let Self {
            config,
            context,
            call,
        } = self;
        match call {
            Call::Request(request) => request_outcome(config, context, request),
            Call::Reply(reply) => reply_outcome(config, reply),
        }
    }
}

fn request_outcome(
    config: &dyn BaseContainerConfig,
    context: ContainerRequestContext<'_>,
    call: RequestCall<'_>,
) -> Result<Outcome, ContainerError> {
    Ok(match call {
        RequestCall::Create { name, params } => {
            Outcome::Create(config.transform_container_create_request(name, params, context))
        }
        RequestCall::List { api_base, query } => {
            let (url, query) = config.transform_container_list_request(api_base, context, &query);
            Outcome::Request { url, query }
        }
        RequestCall::Retrieve {
            container_id,
            api_base,
        } => {
            let (url, query) =
                config.transform_container_retrieve_request(container_id, api_base, context)?;
            Outcome::Request { url, query }
        }
        RequestCall::Delete {
            container_id,
            api_base,
        } => {
            let (url, query) =
                config.transform_container_delete_request(container_id, api_base, context)?;
            Outcome::Request { url, query }
        }
        RequestCall::FileList {
            container_id,
            api_base,
            query,
        } => {
            let (url, query) = config.transform_container_file_list_request(
                container_id,
                api_base,
                context,
                &query,
            )?;
            Outcome::Request { url, query }
        }
        RequestCall::FileContent {
            container_id,
            file_id,
            api_base,
        } => {
            let (url, query) = config.transform_container_file_content_request(
                container_id,
                file_id,
                api_base,
                context,
            )?;
            Outcome::Request { url, query }
        }
    })
}

fn reply_outcome(
    config: &dyn BaseContainerConfig,
    reply: ReplyCall<'_>,
) -> Result<Outcome, ContainerError> {
    Ok(match reply {
        ReplyCall::Create(body) => {
            Outcome::Container(Box::new(config.transform_container_create_response(body)?))
        }
        ReplyCall::List(body) => {
            Outcome::List(Box::new(config.transform_container_list_response(body)?))
        }
        ReplyCall::Retrieve(body) => Outcome::Container(Box::new(
            config.transform_container_retrieve_response(body)?,
        )),
        ReplyCall::Delete(body) => {
            Outcome::Deleted(Box::new(config.transform_container_delete_response(body)?))
        }
        ReplyCall::FileList(body) => Outcome::FileList(Box::new(
            config.transform_container_file_list_response(body)?,
        )),
        ReplyCall::FileContent(body) => Outcome::FileContent(
            config
                .transform_container_file_content_response(body)?
                .into_boxed_slice(),
        ),
    })
}
