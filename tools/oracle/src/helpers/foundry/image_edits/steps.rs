use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::foundry_credentials::Settings as FoundrySettings;
use betterlite_config::types::Config;
use betterlite_foundry::credentials::model_info::ModelInfoAccessors;
use betterlite_foundry::image_edits::auth::{ValidateEnvironmentRequest, validate_environment};
use betterlite_foundry::image_edits::error::ImageEditError;
use betterlite_foundry::image_edits::flux2::{self, Flux2EditRequest};
use betterlite_foundry::image_edits::image_source::ImageSource;
use betterlite_foundry::image_edits::mai::{self, MaiEditRequest};
use betterlite_foundry::image_edits::selection::ImageEditVariant;
use betterlite_foundry::image_edits::url::{CompleteUrlRequest, complete_url};
use betterlite_openai::image_edits::file::ImageEditFile;
use betterlite_openai::image_edits::request::ImageEditForm;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use secrecy::SecretString;
use serde_json::{Value, json};

use super::error_value;
use super::input::{AuthStep, ImageField, ImageKind, TransformStep, UrlStep};

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
        Ok(SecretString::from("entra-fixture-token"))
    }
}

fn load(environment: &BTreeMap<String, String>) -> Result<Config, Box<HelperError>> {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load("")
    .or_config_error()
}

fn model_accessors<'data>(
    settings: &'data FoundrySettings,
    global_api_base: Option<&'data str>,
    global_api_key: Option<&'data SecretString>,
) -> ModelInfoAccessors<'data> {
    ModelInfoAccessors {
        global_api_base,
        global_api_key,
        environment_api_base: settings.api_base.as_deref(),
        environment_api_key: settings.api_key.as_ref(),
    }
}

pub(super) fn auth_step(step: &AuthStep) -> Result<Value, Box<HelperError>> {
    let config = load(&step.environment)?;
    let explicit = step.api_key.clone().map(SecretString::from);
    let global = step.global_api_key.clone().map(SecretString::from);
    let accessors = model_accessors(
        &config.sections.foundry_credentials,
        step.global_api_base.as_deref(),
        global.as_ref(),
    );
    let base = accessors.api_base(step.api_base.as_deref());
    let result = validate_environment(ValidateEnvironmentRequest {
        headers: step.headers.clone(),
        model: &step.model,
        api_key: explicit.as_ref(),
        params: step.params.as_ref(),
        environment: &step.environment,
        accessors,
        refresh_enabled: false,
        sources: &EmptySources,
    });
    Ok(step_value(result.map(|built| {
        json!({
            "class": ImageEditVariant::select(&step.model).map_or("", ImageEditVariant::class_name),
            "multipart": ImageEditVariant::select(&step.model).is_ok_and(ImageEditVariant::uses_multipart),
            "base": base,
            "headers": built,
        })
    })))
}

pub(super) fn url_step(step: &UrlStep) -> Result<Value, Box<HelperError>> {
    let config = load(&step.environment)?;
    let request = url_request(step, &config);
    Ok(step_value(
        complete_url(&request).map(|url| json!({"value": url})),
    ))
}

fn url_request<'data>(step: &'data UrlStep, config: &'data Config) -> CompleteUrlRequest<'data> {
    CompleteUrlRequest {
        model: &step.model,
        accessors: model_accessors(&config.sections.foundry_credentials, None, None),
        api_base: step.api_base.as_deref(),
        api_version: step.api_version.as_deref(),
        global_api_version: step.global_api_version.as_deref(),
        environment: &config.sections.foundry_image_edits,
    }
}

fn step_value(result: Result<Value, ImageEditError>) -> Value {
    match result {
        Ok(value) => value,
        Err(error) => error_value(&error),
    }
}

// a null entry stays in the list, since FLUX 2 counts it and refuses it as an unsupported image
fn sources(step: &TransformStep) -> Option<Vec<ImageSource>> {
    let fields: Vec<Option<&ImageField>> = match (&step.image, &step.images) {
        (_, Some(list)) => list.iter().map(Option::as_ref).collect(),
        (Some(single), None) => vec![Some(single)],
        (None, None) => return None,
    };
    Some(fields.into_iter().map(image_source).collect())
}

fn image_source(field: Option<&ImageField>) -> ImageSource {
    let Some(present) = field else {
        return ImageSource::Unsupported {
            kind: String::from("NoneType"),
        };
    };
    let content = present.text.clone().into_bytes();
    match present.kind {
        ImageKind::Bytes => ImageSource::Bytes(content),
        ImageKind::Buffered => ImageSource::Buffered {
            content,
            position: present.position,
        },
        ImageKind::Reader => ImageSource::Reader {
            content,
            position: present.position,
        },
        ImageKind::Unsupported => ImageSource::Unsupported {
            kind: present.text.clone(),
        },
    }
}

fn mai_file(image: &ImageField) -> ImageEditFile {
    ImageEditFile {
        content: image.text.clone().into_bytes(),
        name: None,
    }
}

pub(super) fn transform_step(step: &TransformStep) -> Value {
    match ImageEditVariant::select(&step.model) {
        Ok(ImageEditVariant::Flux2) => flux2_step(step),
        Ok(ImageEditVariant::Mai) => projected("mai", mai_form(step)),
        Ok(ImageEditVariant::Flux1) => Value::Null,
        Err(_) => json!({"error": {"kind": "ValueError", "status": null, "provider": null}}),
    }
}

fn flux2_step(step: &TransformStep) -> Value {
    // the dispatch maps the optional request params before the transform, so the probe runs the same order
    let mapped = match flux2::map_edit_params(&step.options, &step.model, false) {
        Ok(mapped) => mapped,
        Err(error) => return error_value(&error),
    };
    let mut built = sources(step);
    let result = flux2::transform_request(Flux2EditRequest {
        model: &step.model,
        prompt: step.prompt.as_deref(),
        images: built.as_deref_mut(),
        options: &mapped,
    });
    result.map_or_else(|error| error_value(&error), |form| projected("flux2", form))
}

fn mai_form(step: &TransformStep) -> ImageEditForm {
    let images = mai_images(step);
    mai::transform_request(&MaiEditRequest {
        model: &step.model,
        prompt: step.prompt.as_deref(),
        image: images.as_deref(),
        options: &step.options,
    })
}

// a null entry stays in the list, since MAI uploads the first non-None image and ignores the rest
fn mai_images(step: &TransformStep) -> Option<Vec<Option<ImageEditFile>>> {
    let Some(list) = &step.images else {
        return step.image.as_ref().map(|image| vec![Some(mai_file(image))]);
    };
    Some(list.iter().map(Option::as_ref).map(mai_file).collect())
}

fn projected(kind: &str, form: ImageEditForm) -> Value {
    let files: Vec<Value> = form
        .files
        .iter()
        .map(|(field, file)| {
            json!({"field": field, "filename": file.filename, "content_type": file.content_type})
        })
        .collect();
    json!({"value": {"kind": kind, "data": form.data, "files": files}})
}
