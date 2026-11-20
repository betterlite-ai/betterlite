use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use secrecy::SecretString;

use super::config::{Headers, Params};
use super::error::VideoEditTransportError;
use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::vector_store_http_write::service::SuppliedClient;
use crate::api::video_records::VideoObject;

// One edit call's arguments, as LiteLLM's video_edit_handler takes them
pub struct VideoEditRequest<'call, Client> {
    pub prompt: &'call str,
    pub video_id: &'call str,
    pub provider: &'call str,
    pub litellm_params: &'call Params,
    pub video_file: Option<&'call SuppliedMedia>,
    pub extra_headers: Option<&'call Headers>,
    pub extra_body: Option<&'call Params>,
    pub timeout: Option<Duration>,
    pub client: Option<SuppliedClient<'call, Client>>,
    pub api_key: Option<&'call SecretString>,
    pub is_async: bool,
}

// One extension call's arguments, as LiteLLM's video_extension_handler takes them
pub struct VideoExtensionRequest<'call, Client> {
    pub prompt: &'call str,
    pub video_id: &'call str,
    pub seconds: &'call str,
    pub provider: &'call str,
    pub litellm_params: &'call Params,
    pub extra_headers: Option<&'call Headers>,
    pub extra_body: Option<&'call Params>,
    pub timeout: Option<Duration>,
    pub client: Option<SuppliedClient<'call, Client>>,
    pub api_key: Option<&'call SecretString>,
    pub is_async: bool,
}

pub type VideoPending<'call> =
    Pin<Box<dyn Future<Output = Result<VideoObject, VideoEditTransportError>> + Send + 'call>>;

// The sync handler answers with the object (boxed, to match the pending variant) or the async handler's future
pub enum Handled<'call> {
    Done(Box<VideoObject>),
    Pending(VideoPending<'call>),
}
