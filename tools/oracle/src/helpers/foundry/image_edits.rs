use std::sync::Arc;

use betterlite_azure::credentials::source::{
    CallableTokenProvider, ClientSecretRecord, CredentialSources, OidcExchangeInput, SourceFailure,
    UsernamePasswordRecord,
};
use betterlite_config::loader::ConfigLoader;
use betterlite_foundry::credentials::model_info::ModelInfoAccessors;
use betterlite_foundry::image_edits::auth::{ValidateEnvironmentRequest, validate_environment};
use betterlite_foundry::image_edits::error::ImageEditError;
use betterlite_foundry::image_edits::flux2::{self, Flux2EditRequest};
use betterlite_foundry::image_edits::mai::{self, MaiEditRequest};
use betterlite_foundry::image_edits::reply::transform_mai_edit_response;
use betterlite_foundry::image_edits::selection::ImageEditVariant;
use betterlite_foundry::image_edits::url::{CompleteUrlRequest, complete_url};
use betterlite_foundry::image_generation::error::FoundryImageError;
use betterlite_foundry::mai_images::error::MaiImageError;
use betterlite_foundry::mai_images::reply::{ImageLogging, PostCall};
use betterlite_openai::image_edits::file::ImageEditFile;
use betterlite_openai::openai_base_config::Headers;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value, json};

mod input;
mod tests;

use input::{AuthStep, ImageField, ImageKind, Input, ReplyStep, TransformStep, UrlStep};

const CLOCK: i64 = 1_700_000_000;
const ENTRA_TOKEN: &str = "entra-fixture-token";

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answer = Map::new();
        if let Some(models) = &input.models {
            answer.insert(
                "models".to_owned(),
                json!(models.iter().map(model_step).collect::<Vec<_>>()),
            );
        }
        if let Some(models) = &input.supported {
            answer.insert(
                "supported".to_owned(),
                json!(models.iter().map(supported_step).collect::<Vec<_>>()),
            );
        }
        if let Some(auths) = &input.auths {
            answer.insert(
                "auths".to_owned(),
                json!(auths.iter().map(auth_step).collect::<Result<Vec<_>, _>>()?),
            );
        }
        if let Some(urls) = &input.urls {
            answer.insert(
                "urls".to_owned(),
                json!(urls.iter().map(url_step).collect::<Result<Vec<_>, _>>()?),
            );
        }
        if let Some(transforms) = &input.transforms {
            answer.insert(
                "transforms".to_owned(),
                json!(transforms.iter().map(transform_step).collect::<Vec<_>>()),
            );
        }
        if let Some(replies) = &input.replies {
            answer.insert(
                "replies".to_owned(),
                json!(replies.iter().map(reply_step).collect::<Vec<_>>()),
            );
        }
        Ok(Value::Object(answer))
    }
}

fn model_step(model: &str) -> Value {
    ImageEditVariant::select(model).map_or_else(
        |_| failure("ValueError", None, None),
        |variant| json!({"class": variant.class_name(), "multipart": variant.uses_multipart()}),
    )
}

// the FLUX 1 allowlist belongs to the OpenAI image edit config, so only the two owned allowlists are probed
fn supported_step(model: &str) -> Value {
    match ImageEditVariant::select(model) {
        Ok(ImageEditVariant::Flux2) => json!(flux2::supported_params(model)),
        Ok(ImageEditVariant::Mai) => json!(mai::supported_params()),
        Ok(ImageEditVariant::Flux1) | Err(_) => Value::Null,
    }
}

struct EmptySources;

fn no_source() -> SourceFailure {
    SourceFailure {
        class: String::from("ValueError"),
        status: None,
        message: String::from("this fixture resolves no credential sources"),
    }
}

impl CredentialSources for EmptySources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(no_source())
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(no_source())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(no_source())
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Ok(SecretString::from(ENTRA_TOKEN))
    }
}

fn auth_step(step: &AuthStep) -> Result<Value, Box<HelperError>> {
    let config = ConfigLoader {
        lookup: step.environment.clone(),
    }
    .load("")
    .or_config_error()?;
    let settings = &config.sections.foundry_credentials;
    let explicit = step.api_key.clone().map(SecretString::from);
    let global = step.global_api_key.clone().map(SecretString::from);
    let accessors = ModelInfoAccessors {
        global_api_base: step.global_api_base.as_deref(),
        global_api_key: global.as_ref(),
        environment_api_base: settings.api_base.as_deref(),
        environment_api_key: settings.api_key.as_ref(),
    };
    let base = accessors.api_base(step.api_base.as_deref());
    let headers: Headers = step
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let result = validate_environment(ValidateEnvironmentRequest {
        headers,
        model: &step.model,
        api_key: explicit.as_ref(),
        params: step.params.as_ref(),
        environment: &step.environment,
        accessors,
        refresh_enabled: false,
        sources: &EmptySources,
    });
    Ok(match result {
        Ok(headers) => json!({
            "class": ImageEditVariant::select(&step.model).map_or("", ImageEditVariant::class_name),
            "multipart": ImageEditVariant::select(&step.model).is_ok_and(ImageEditVariant::uses_multipart),
            "base": base,
            "headers": headers,
        }),
        Err(error) => error_value(&error),
    })
}

fn url_step(step: &UrlStep) -> Result<Value, Box<HelperError>> {
    let config = ConfigLoader {
        lookup: step.environment.clone(),
    }
    .load("")
    .or_config_error()?;
    let settings = &config.sections.foundry_credentials;
    let accessors = ModelInfoAccessors {
        global_api_base: None,
        global_api_key: None,
        environment_api_base: settings.api_base.as_deref(),
        environment_api_key: settings.api_key.as_ref(),
    };
    let result = complete_url(&CompleteUrlRequest {
        model: &step.model,
        accessors,
        api_base: step.api_base.as_deref(),
        api_version: step.api_version.as_deref(),
        global_api_version: step.global_api_version.as_deref(),
        environment: &config.sections.foundry_image_edits,
    });
    Ok(match result {
        Ok(url) => json!({"value": url}),
        Err(error) => error_value(&error),
    })
}

fn sources(
    step: &TransformStep,
) -> Option<Vec<betterlite_foundry::image_edits::image_source::ImageSource>> {
    let fields: Vec<&ImageField> = match (&step.image, &step.images) {
        (_, Some(list)) => list.iter().flatten().collect(),
        (Some(single), None) => vec![single],
        (None, None) => return None,
    };
    Some(fields.iter().map(|field| image_source(field)).collect())
}

fn image_source(field: &ImageField) -> betterlite_foundry::image_edits::image_source::ImageSource {
    use betterlite_foundry::image_edits::image_source::ImageSource;
    let content = field.text.clone().into_bytes();
    match field.kind {
        ImageKind::Bytes => ImageSource::Bytes(content),
        ImageKind::Buffered => ImageSource::Buffered {
            content,
            position: field.position,
        },
        ImageKind::Reader => ImageSource::Reader {
            content,
            position: field.position,
        },
        ImageKind::Unsupported => ImageSource::Unsupported {
            kind: field.text.clone(),
        },
    }
}

fn transform_step(step: &TransformStep) -> Value {
    let Ok(variant) = ImageEditVariant::select(&step.model) else {
        return failure("ValueError", None, None);
    };
    match variant {
        ImageEditVariant::Flux2 => {
            // the dispatch maps the optional request params before the transform, so the probe runs the same order
            let mapped = match flux2::map_edit_params(&step.options, &step.model, false) {
                Ok(mapped) => mapped,
                Err(error) => return error_value(&ImageEditError::Params(error)),
            };
            let mut built = sources(step);
            let result = flux2::transform_request(Flux2EditRequest {
                model: &step.model,
                prompt: step.prompt.as_deref(),
                images: built.as_mut().map(Vec::as_mut_slice),
                options: &mapped,
            });
            match result {
                Ok(form) => json!({"kind": "flux2", "data": form.data, "files": []}),
                Err(error) => error_value(&error),
            }
        }
        ImageEditVariant::Mai => {
            let files: Option<Vec<Option<ImageEditFile>>> = step.images.as_ref().map(|list| {
                list.iter()
                    .map(|field| {
                        field.as_ref().map(|field| ImageEditFile {
                            content: field.text.clone().into_bytes(),
                            name: None,
                        })
                    })
                    .collect()
            });
            let single_files: Option<Vec<Option<ImageEditFile>>> =
                step.image.as_ref().map(|field| {
                    vec![Some(ImageEditFile {
                        content: field.text.clone().into_bytes(),
                        name: None,
                    })]
                });
            let form = mai::transform_request(MaiEditRequest {
                model: &step.model,
                prompt: step.prompt.as_deref(),
                image: files.as_deref().or(single_files.as_deref()),
                options: &step.options,
            });
            let files: Vec<Value> = form
                .files
                .iter()
                .map(|(field, file)| {
                    json!({
                        "field": field,
                        "filename": file.filename,
                        "content_type": file.content_type,
                        "text": String::from_utf8_lossy(&file.content),
                    })
                })
                .collect();
            json!({"kind": "mai", "data": form.data, "files": files})
        }
        ImageEditVariant::Flux1 => Value::Null,
    }
}

#[derive(Default)]
struct Recorder {
    response: Option<Value>,
}

impl ImageLogging for Recorder {
    fn post_call(&mut self, call: PostCall<'_>) {
        self.response = Some(json!({
            "input": call.input,
            "api_key": call.api_key.map(<SecretString as ExposeSecret<str>>::expose_secret),
            "original_response": call.original_response,
            "complete_input_dict": call.complete_input_dict,
        }));
    }
}

fn reply_step(step: &ReplyStep) -> Value {
    let mut recorder = Recorder::default();
    let result =
        transform_mai_edit_response(step.body.as_bytes(), step.status, CLOCK, &mut recorder);
    match result {
        Ok(reply) => {
            // only the present image fields are projected, so an absent one compares like LiteLLM's exclude_none
            let data: Vec<Value> = reply
                .data
                .iter()
                .map(|datum| {
                    let mut fields = Map::new();
                    if let Some(value) = &datum.b64_json {
                        fields.insert(String::from("b64_json"), json!(value));
                    }
                    if let Some(value) = &datum.url {
                        fields.insert(String::from("url"), json!(value));
                    }
                    if let Some(value) = &datum.revised_prompt {
                        fields.insert(String::from("revised_prompt"), json!(value));
                    }
                    Value::Object(fields)
                })
                .collect();
            json!({
                "created": reply.created,
                "data": data,
                "size": reply.size,
                "usage": {
                    "input_tokens": reply.usage.input_tokens,
                    "input_tokens_details": {
                        "image_tokens": reply.usage.input_tokens_details.image_tokens,
                        "text_tokens": reply.usage.input_tokens_details.text_tokens,
                    },
                    "output_tokens": reply.usage.output_tokens,
                    "total_tokens": reply.usage.total_tokens,
                },
                "logged": recorder.response,
            })
        }
        Err(error) => error_value(&error),
    }
}

// the exception LiteLLM raises for each betterlite error, so the oracle compares the branch, not the text
fn error_value(error: &ImageEditError) -> Value {
    match error {
        ImageEditError::Params(inner) => params_failure(inner),
        ImageEditError::Mai(MaiImageError::NotJson { record, .. }) => {
            failure("OpenAIError", Some(record.status_code), None)
        }
        ImageEditError::Mai(_) | ImageEditError::Projection(_) => {
            failure("ValueError", error.status_code(), None)
        }
        ImageEditError::UnsupportedModel { .. }
        | ImageEditError::ApiBaseRequired
        | ImageEditError::ApiVersionRequired
        | ImageEditError::Flux2PromptRequired
        | ImageEditError::Flux2ImageRequired
        | ImageEditError::Flux2NoReferenceImages
        | ImageEditError::Flux2TooManyReferenceImages { .. }
        | ImageEditError::UnsupportedImageType { .. }
        | ImageEditError::Url(_)
        | ImageEditError::Credentials(_)
        | ImageEditError::ReplyEncoding(_) => failure("ValueError", None, None),
    }
}

fn params_failure(error: &FoundryImageError) -> Value {
    let kind = match error {
        FoundryImageError::InvalidSize { .. } => "BadRequestError",
        FoundryImageError::UnsupportedParams { .. } => "UnsupportedParamsError",
        FoundryImageError::ApiBaseRequired
        | FoundryImageError::WholeNumber { .. }
        | FoundryImageError::Decimal { .. }
        | FoundryImageError::GptParams(_) => "ValueError",
    };
    failure(kind, error.status_code(), error.llm_provider())
}

fn failure(kind: &str, status: Option<u16>, provider: Option<&str>) -> Value {
    json!({"error": {"kind": kind, "status": status, "provider": provider}})
}
