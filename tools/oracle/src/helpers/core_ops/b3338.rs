use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_generation_call::VideoGenerationRequest;
use betterlite_core::api::video_generation_call::call::mock_video_response;
use betterlite_core::api::video_generation_call::options::{
    provider_request_params, requested_optional_params,
};
use betterlite_core::api::video_generation_call::providers::VideoGenerationConfig;
// ast-grep-ignore: no-long-names
use betterlite_core::api::video_records::{VideoCreateOptionalRequestParams, VideoObject};
use betterlite_core::api::videos_http_transport::video_config::{
    Params, RequestFiles, VideoHttpConfig, VideoLogging, VideoReply, VideoRequestContext,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Number;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Requested {
        call: CallInput,
    },
    Provider {
        call: CallInput,
        model: String,
        drop_params: bool,
    },
    Mock {
        mock_response: String,
    },
}

// video_generation's named arguments and its loose keywords, every field present in the case
#[derive(Debug, Deserialize)]
pub(crate) struct CallInput {
    prompt: String,
    model: Option<String>,
    seconds: Option<String>,
    size: Option<String>,
    user: Option<String>,
    custom_llm_provider: Option<String>,
    extra_headers: Option<IndexMap<String, String>>,
    extra_query: Option<Params>,
    extra_body: Option<Params>,
    timeout: Option<f64>,
    kwargs: Params,
}

impl CallInput {
    fn request(&self) -> VideoGenerationRequest {
        VideoGenerationRequest {
            model: self.model.clone(),
            seconds: self.seconds.clone(),
            size: self.size.clone(),
            user: self.user.clone(),
            timeout: self.timeout,
            custom_llm_provider: self.custom_llm_provider.clone(),
            extra_headers: self.extra_headers.clone(),
            extra_query: self.extra_query.clone(),
            extra_body: self.extra_body.clone(),
            kwargs: self.kwargs.clone(),
            ..VideoGenerationRequest::new(self.prompt.clone())
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Requested { call } => {
                let mut request = call.request();
                let options = requested_optional_params(&mut request).or_value_error()?;
                let mut params = typed_params(&options.video);
                if let Some(query) = options.extra_query {
                    params.insert("extra_query".to_owned(), Json::Object(query));
                }
                serde_json::to_value(params).or_value_error()
            }
            Input::Provider {
                call,
                model,
                drop_params,
            } => {
                let mut request = call.request();
                let options = requested_optional_params(&mut request).or_value_error()?;
                let params = provider_request_params(&Identity, model, *drop_params, &options);
                serde_json::to_value(&params).or_value_error()
            }
            Input::Mock { mock_response } => {
                let kwargs = Params::from([(
                    "mock_response".to_owned(),
                    Json::Text(mock_response.clone()),
                )]);
                mock_video_response(&kwargs)
                    .or_value_error()?
                    .map_or(Ok(Value::Null), |object| {
                        serde_json::to_value(&object).or_value_error()
                    })
            }
        }
    }
}

// the typed options as the dict get_requested_video_generation_optional_param answers: only the present keys
fn typed_params(
    // ast-grep-ignore: no-long-names
    options: &VideoCreateOptionalRequestParams,
) -> Params {
    let mut params = Params::new();
    if let Some(media) = &options.input_reference {
        params.insert("input_reference".to_owned(), media_json(media));
    }
    if let Some(image) = &options.image {
        params.insert("image".to_owned(), image.clone());
    }
    if let Some(parameters) = &options.parameters {
        params.insert("parameters".to_owned(), Json::Object(parameters.clone()));
    }
    for (name, value) in [
        ("model", &options.model),
        ("resolution", &options.resolution),
        ("seconds", &options.seconds),
        ("size", &options.size),
        ("user", &options.user),
    ] {
        if let Some(text) = value {
            params.insert(name.to_owned(), Json::Text(text.clone()));
        }
    }
    if let Some(characters) = &options.characters {
        params.insert(
            "characters".to_owned(),
            Json::List(characters.iter().map(text_map_json).collect()),
        );
    }
    if let Some(headers) = &options.extra_headers {
        params.insert("extra_headers".to_owned(), text_map_json(headers));
    }
    if let Some(extra_body) = &options.extra_body {
        params.insert("extra_body".to_owned(), text_map_json(extra_body));
    }
    params
}

fn text_map_json(entries: &IndexMap<String, String>) -> Json {
    Json::Object(
        entries
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect(),
    )
}

fn media_json(media: &SuppliedMedia) -> Json {
    match media {
        SuppliedMedia::Text(text) => Json::Text(text.clone()),
        SuppliedMedia::List(items) => Json::List(items.iter().map(media_json).collect()),
        SuppliedMedia::Bytes(bytes) => Json::List(
            bytes
                .iter()
                .map(|byte| Json::Number(Number::from(*byte)))
                .collect(),
        ),
        SuppliedMedia::File(file) => media_json(&SuppliedMedia::Bytes(file.content.clone())),
    }
}

// OpenAIVideoConfig.map_openai_params answers the options unchanged; the helper never sends a request
struct Identity;

#[derive(Debug, Error)]
#[error("the b3338 oracle config never sends a request")]
struct NeverSent;

impl VideoGenerationConfig for Identity {
    fn map_openai_params(
        &self,
        // ast-grep-ignore: no-long-names
        options: &VideoCreateOptionalRequestParams,
        _model: &str,
        _drop_params: bool,
    ) -> Params {
        typed_params(options)
    }
}

macro_rules! never_sent {
    ($(fn $name:ident($($argument:ty),* $(,)?) -> $output:ty;)*) => {
        $(
            fn $name(&self, $(_: $argument),*) -> $output {
                Err(NeverSent)
            }
        )*
    };
}

impl VideoHttpConfig for Identity {
    type Error = NeverSent;

    never_sent! {
        fn get_complete_url(&str, Option<&str>, &Params) -> Result<String, Self::Error>;
        fn transform_video_create_request(&str, &str, &Params, VideoRequestContext<'_>) -> Result<(Params, RequestFiles, String), Self::Error>;
        // ast-grep-ignore: no-long-names
        fn transform_video_create_response(&str, VideoReply<'_>, Option<&Params>) -> Result<VideoObject, Self::Error>;
        // ast-grep-ignore: no-long-names
        fn transform_video_content_request(&str, Option<&str>, VideoRequestContext<'_>) -> Result<(String, Params), Self::Error>;
        // ast-grep-ignore: no-long-names
        fn transform_video_content_response(&UpstreamResponse, &mut VideoLogging) -> Result<Vec<u8>, Self::Error>;
    }
}

mod tests;
