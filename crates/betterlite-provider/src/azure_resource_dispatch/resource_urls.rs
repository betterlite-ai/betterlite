use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;

use betterlite_azure::resource_urls::error::ResourceUrlError;
use betterlite_azure::resource_urls::{
    AzureResource, ResourceAuth, UrlRequest, VIDEO_SUPPORTED_PARAMS, complete_url,
    validate_environment, video_map_params, video_validate_environment,
};
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::create::VideoCreate;
use secrecy::{ExposeSecret as _, SecretString};

pub struct Dispatch<'call> {
    pub call: Call<'call>,
}

// B547 overrides only the URL, the auth and the video parameter list; every OpenAI request transform stays with its leaf
pub enum Call<'call> {
    Url {
        resource: AzureResource,
        request: UrlRequest<'call>,
    },
    Environment {
        auth: ResourceAuth<'call>,
        params_api_key: Option<&'call SecretString>,
    },
    VideoEnvironment {
        auth: ResourceAuth<'call>,
        api_key: Option<&'call SecretString>,
        params_api_key: &'call mut Option<SecretString>,
    },
    VideoParams(&'call VideoCreate),
    VideoSupported,
}

pub enum Outcome {
    Url(String),
    Headers(BTreeMap<String, String>),
    VideoHeaders {
        headers: BTreeMap<String, String>,
        params_api_key: Option<String>,
    },
    VideoParams(Box<VideoOptions>),
    VideoSupported(&'static [&'static str]),
}

pub type RunFuture<'call> =
    Pin<Box<dyn Future<Output = Result<Outcome, ResourceUrlError>> + Send + 'call>>;

impl Dispatch<'_> {
    // Every container, video and vector-store override B547 exposes, plus its delegated video mapping
    pub fn run(self) -> Result<Outcome, ResourceUrlError> {
        Ok(match self.call {
            Call::Url { resource, request } => Outcome::Url(complete_url(resource, &request)?),
            Call::Environment {
                auth,
                params_api_key,
            } => Outcome::Headers(validate_environment(auth, params_api_key)?),
            Call::VideoEnvironment {
                auth,
                api_key,
                params_api_key,
            } => {
                let headers = video_validate_environment(auth, api_key, params_api_key)?;
                Outcome::VideoHeaders {
                    headers,
                    params_api_key: params_api_key
                        .as_ref()
                        .map(|key| key.expose_secret().to_owned()),
                }
            }
            Call::VideoParams(options) => Outcome::VideoParams(Box::new(video_map_params(options))),
            Call::VideoSupported => Outcome::VideoSupported(&VIDEO_SUPPORTED_PARAMS),
        })
    }
}

pub fn dispatch(dispatch: Dispatch<'_>) -> RunFuture<'_> {
    Box::pin(ready(dispatch.run()))
}
