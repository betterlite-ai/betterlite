use std::sync::Arc;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::loader::ConfigLoader;
use betterlite_foundry::credentials::model_info::ModelInfoAccessors;
use betterlite_foundry::image_edits::auth::{ValidateEnvironmentRequest, validate_environment};
use betterlite_foundry::image_edits::flux2::{self, Flux2EditRequest};
use betterlite_foundry::image_edits::image_source::ImageSource;
use betterlite_foundry::image_edits::mai::{self, MaiEditRequest};
use betterlite_foundry::image_edits::selection::ImageEditVariant;
use betterlite_foundry::image_edits::url::{CompleteUrlRequest, complete_url};
use betterlite_openai::image_edits::file::ImageEditFile;
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

pub(super) fn auth_step(step: &AuthStep) -> Result<Value, Box<HelperError>> {
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
    Ok(match result {
        Ok(built) => json!({
            "class": ImageEditVariant::select(&step.model).map_or("", ImageEditVariant::class_name),
            "multipart": ImageEditVariant::select(&step.model).is_ok_and(ImageEditVariant::uses_multipart),
            "base": base,
            "headers": built,
        }),
        Err(error) => error_value(&error),
    })
}

pub(super) fn url_step(step: &UrlStep) -> Result<Value, Box<HelperError>> {
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

fn mai_file(field: Option<&ImageField>) -> Option<ImageEditFile> {
    field.map(|image| ImageEditFile {
        content: image.text.clone().into_bytes(),
        name: None,
    })
}

pub(super) fn transform_step(step: &TransformStep) -> Value {
    match ImageEditVariant::select(&step.model) {
        Ok(ImageEditVariant::Flux2) => flux2_step(step),
        Ok(ImageEditVariant::Mai) => mai_step(step),
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
        images: built.as_mut().map(Vec::as_mut_slice),
        options: &mapped,
    });
    match result {
        Ok(form) => json!({"value": {"kind": "flux2", "data": form.data, "files": []}}),
        Err(error) => error_value(&error),
    }
}

fn mai_step(step: &TransformStep) -> Value {
    let files: Option<Vec<Option<ImageEditFile>>> = step
        .images
        .as_ref()
        .map(|list| list.iter().map(Option::as_ref).map(mai_file).collect());
    let single_files = step.image.as_ref().map(|image| vec![mai_file(Some(image))]);
    let form = mai::transform_request(&MaiEditRequest {
        model: &step.model,
        prompt: step.prompt.as_deref(),
        image: files.as_deref().or(single_files.as_deref()),
        options: &step.options,
    });
    let projected: Vec<Value> = form
        .files
        .iter()
        .map(|(field, file)| {
            json!({
                "field": field,
                "filename": file.filename,
                "content_type": file.content_type,
            })
        })
        .collect();
    json!({"value": {"kind": "mai", "data": form.data, "files": projected}})
}
