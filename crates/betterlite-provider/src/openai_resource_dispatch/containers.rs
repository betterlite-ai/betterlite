use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::api::container_contracts::error::ContainerError;
use betterlite_openai::openai_base_config::Object;

pub struct Dispatch<'a> {
    pub config: &'a dyn BaseContainerConfig,
    pub context: ContainerRequestContext<'a>,
    pub call: Call<'a>,
}

// B522's six request operations plus the file-content decode that keeps the bytes intact
pub enum Call<'a> {
    Create {
        name: &'a str,
        params: &'a Object,
    },
    List {
        api_base: &'a str,
        query: ContainerListQuery<'a>,
    },
    Retrieve {
        id: &'a str,
        api_base: &'a str,
    },
    Delete {
        id: &'a str,
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
    FileContentReply(&'a [u8]),
}

#[derive(Debug)]
pub enum Outcome {
    Create(Object),
    Request { url: String, query: Object },
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
            Call::Create { name, params } => Ok(Outcome::Create(
                config.transform_container_create_request(name, params, context),
            )),
            Call::List { api_base, query } => {
                let (url, query) =
                    config.transform_container_list_request(api_base, context, &query);
                Ok(Outcome::Request { url, query })
            }
            Call::Retrieve { id, api_base } => retrieve_request(config, context, id, api_base),
            Call::Delete { id, api_base } => delete_request(config, context, id, api_base),
            Call::FileList {
                container_id,
                api_base,
                query,
            } => file_list_request(config, context, container_id, api_base, &query),
            Call::FileContent {
                container_id,
                file_id,
                api_base,
            } => file_content_request(config, context, container_id, file_id, api_base),
            Call::FileContentReply(body) => Ok(Outcome::FileContent(
                config
                    .transform_container_file_content_response(body)?
                    .into_boxed_slice(),
            )),
        }
    }
}

fn retrieve_request(
    config: &dyn BaseContainerConfig,
    context: ContainerRequestContext<'_>,
    container_id: &str,
    api_base: &str,
) -> Result<Outcome, ContainerError> {
    let (url, query) =
        config.transform_container_retrieve_request(container_id, api_base, context)?;
    Ok(Outcome::Request { url, query })
}

fn delete_request(
    config: &dyn BaseContainerConfig,
    context: ContainerRequestContext<'_>,
    container_id: &str,
    api_base: &str,
) -> Result<Outcome, ContainerError> {
    let (url, query) =
        config.transform_container_delete_request(container_id, api_base, context)?;
    Ok(Outcome::Request { url, query })
}

fn file_list_request(
    config: &dyn BaseContainerConfig,
    context: ContainerRequestContext<'_>,
    container_id: &str,
    api_base: &str,
    query: &ContainerListQuery<'_>,
) -> Result<Outcome, ContainerError> {
    let (url, query) =
        config.transform_container_file_list_request(container_id, api_base, context, query)?;
    Ok(Outcome::Request { url, query })
}

fn file_content_request(
    config: &dyn BaseContainerConfig,
    context: ContainerRequestContext<'_>,
    container_id: &str,
    file_id: &str,
    api_base: &str,
) -> Result<Outcome, ContainerError> {
    let (url, query) = config.transform_container_file_content_request(
        container_id,
        file_id,
        api_base,
        context,
    )?;
    Ok(Outcome::Request { url, query })
}
