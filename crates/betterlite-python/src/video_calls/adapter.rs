mod remix_list;
mod status;
mod support;

use std::collections::BTreeMap;

use betterlite_core::api::video_generation_call::providers::VideoGenerationConfig;
// ast-grep-ignore: no-long-names
use betterlite_core::api::video_records::{VideoCreateOptionalRequestParams, VideoObject};
use betterlite_core::api::videos_http_transport::video_config::{
    Headers, Params, RequestFiles, VideoEnvironment, VideoHttpConfig, VideoLogging, VideoReply,
    VideoRequestContext,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::multipart::MediaBody;
use betterlite_openai::videos::config::{VideoCredentials, complete_url, validate_environment};
use betterlite_openai::videos::content::content_request;
use betterlite_openai::videos::create::{VideoCreate, create_request, create_response};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use self::support::{file_part, params_key};
use super::convert::ConvertError;
use super::convert::Reference;
use super::convert::fields::json;

// LiteLLM's OpenAIVideoConfig over B525's free functions: the same identity every video operation reads
pub(crate) struct OpenAiVideoConfig {
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) global_api_key: Option<SecretString>,
    pub(crate) openai_key: Option<SecretString>,
    pub(crate) reference: Option<Reference>,
}

impl OpenAiVideoConfig {
    // validate_environment: api_key, then the params' key, then litellm.api_key, litellm.openai_key and OPENAI_API_KEY
    pub(crate) fn headers(
        &self,
        given: &IndexMap<String, String>,
        api_key: Option<&SecretString>,
        params: Option<&IndexMap<String, Json>>,
    ) -> IndexMap<String, String> {
        let credentials = VideoCredentials {
            api_key: api_key.cloned(),
            params_api_key: params.and_then(params_key),
            global_api_key: self.global_api_key.clone(),
            openai_key: self.openai_key.clone(),
            environment_api_key: self
                .environment
                .get("OPENAI_API_KEY")
                .filter(|value| !value.is_empty())
                .map(|value| SecretString::from(value.clone())),
        };
        let input = given
            .iter()
            .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
            .collect();
        validate_environment(input, &credentials)
            .into_iter()
            .map(|(name, value)| (name, value.expose_secret().to_owned()))
            .collect()
    }

    // the one file a generation sends: the caller's input_reference, under the name the payload gave it
    fn create_form(
        &self,
        model: &str,
        prompt: &str,
        optional_params: &Params,
        api_base: &str,
    ) -> Result<(Params, RequestFiles, String), ConvertError> {
        let options: VideoCreate = json(optional_params)?;
        let media = self
            .reference
            .as_ref()
            .map(|reference| MediaBody::Bytes(reference.content.as_slice()));
        let form = create_request(model, prompt, api_base, &options, media);
        let name = self
            .reference
            .as_ref()
            .and_then(|reference| reference.name.clone());
        let mut files = Vec::new();
        for part in form.files {
            files.push(file_part(part, name.clone())?);
        }
        Ok((json(&form.fields)?, files, form.api_base))
    }
}

impl VideoHttpConfig for OpenAiVideoConfig {
    type Error = ConvertError;

    fn validate_environment(&self, request: VideoEnvironment<'_>) -> Headers {
        self.headers(request.headers, request.api_key, request.litellm_params)
    }

    fn get_complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, Self::Error> {
        Ok(complete_url(api_base))
    }

    fn use_multipart_form_data(&self) -> bool {
        true
    }

    fn transform_video_create_request(
        &self,
        model: &str,
        prompt: &str,
        optional_params: &Params,
        context: VideoRequestContext<'_>,
    ) -> Result<(Params, RequestFiles, String), Self::Error> {
        self.create_form(model, prompt, optional_params, context.api_base)
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_create_response(
        &self,
        model: &str,
        reply: VideoReply<'_>,
        _request_data: Option<&Params>,
    ) -> Result<VideoObject, Self::Error> {
        let video = create_response(model, &reply.raw_response.body, reply.custom_llm_provider)?;
        json(&video)
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_content_request(
        &self,
        video_id: &str,
        variant: Option<&str>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), Self::Error> {
        let (url, data) = content_request(video_id, context.api_base, variant)?;
        Ok((url, json(&data)?))
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_content_response(
        &self,
        raw_response: &UpstreamResponse,
        _logging_obj: &mut VideoLogging,
    ) -> Result<Vec<u8>, Self::Error> {
        Ok(raw_response.body.clone())
    }
}

impl VideoGenerationConfig for OpenAiVideoConfig {
    // map_openai_params hands the declared options on unchanged, as OpenAIVideoConfig does
    fn map_openai_params(
        &self,
        // ast-grep-ignore: no-long-names
        options: &VideoCreateOptionalRequestParams,
        _model: &str,
        _drop_params: bool,
    ) -> Params {
        let mut params = Params::new();
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
        if let Some(image) = &options.image {
            params.insert("image".to_owned(), image.clone());
        }
        if let Some(parameters) = &options.parameters {
            params.insert(
                "parameters".to_owned(),
                Json::Object(parameters.clone().into_iter().collect()),
            );
        }
        if let Some(characters) = &options.characters {
            params.insert(
                "characters".to_owned(),
                Json::List(characters.iter().map(text_map).collect()),
            );
        }
        if let Some(headers) = &options.extra_headers {
            params.insert("extra_headers".to_owned(), text_map(headers));
        }
        if let Some(body) = &options.extra_body {
            params.insert("extra_body".to_owned(), text_map(body));
        }
        params
    }
}

fn text_map(entries: &IndexMap<String, String>) -> Json {
    Json::Object(
        entries
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect(),
    )
}

mod tests;
