use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::api::container_contracts::error::ContainerError;
use betterlite_openai::openai_base_config::Object;

pub struct Dispatch<'call> {
    pub config: &'call dyn BaseContainerConfig,
    pub context: ContainerRequestContext<'call>,
    pub call: Call<'call>,
}

// B522's six request operations plus the file-content decode that keeps the bytes intact
pub enum Call<'call> {
    Create {
        name: &'call str,
        params: &'call Object,
    },
    List {
        api_base: &'call str,
        query: ContainerListQuery<'call>,
    },
    Retrieve {
        id: &'call str,
        api_base: &'call str,
    },
    Delete {
        id: &'call str,
        api_base: &'call str,
    },
    FileList {
        container_id: &'call str,
        api_base: &'call str,
        query: ContainerListQuery<'call>,
    },
    FileContent {
        container_id: &'call str,
        file_id: &'call str,
        api_base: &'call str,
    },
    FileContentReply(&'call [u8]),
}

#[derive(Debug)]
pub enum Outcome {
    Create(Object),
    Request((String, Object)),
    FileContent(Box<[u8]>),
}

impl Dispatch<'_> {
    pub fn run(self) -> Result<Outcome, ContainerError> {
        let Self {
            config,
            context,
            call,
        } = self;
        Ok(match call {
            Call::Create { name, params } => {
                Outcome::Create(config.transform_container_create_request(name, params, context))
            }
            Call::List { api_base, query } => {
                Outcome::Request(config.transform_container_list_request(api_base, context, &query))
            }
            Call::Retrieve { id, api_base } => Outcome::Request(
                config.transform_container_retrieve_request(id, api_base, context)?,
            ),
            Call::Delete { id, api_base } => {
                Outcome::Request(config.transform_container_delete_request(id, api_base, context)?)
            }
            Call::FileList {
                container_id,
                api_base,
                query,
            } => Outcome::Request(config.transform_container_file_list_request(
                container_id,
                api_base,
                context,
                &query,
            )?),
            Call::FileContent {
                container_id,
                file_id,
                api_base,
            } => Outcome::Request(config.transform_container_file_content_request(
                container_id,
                file_id,
                api_base,
                context,
            )?),
            Call::FileContentReply(body) => Outcome::FileContent(
                config
                    .transform_container_file_content_response(body)?
                    .into_boxed_slice(),
            ),
        })
    }
}
