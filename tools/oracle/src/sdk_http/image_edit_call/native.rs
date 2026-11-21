use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::image_edit_call::call::{ImageEditCall, ImageEditClientKind, ImageInput};
use betterlite_core::api::image_edit_call::entry::ImageEditEntry;
use betterlite_core::api::image_edit_call::error::ImageEditException;
use betterlite_core::api::image_edit_call::logging::{ImageEditLogger, PreCallRecord};
use betterlite_core::api::image_edit_call::service::ImageEdits;
use betterlite_error::envelope::WireField;
use betterlite_error::error::class_message;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::image_contracts::records::{ImageFile, ImageResponse};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_wrapper::error::class_name;

use super::super::error::SdkHttpError;
use super::super::image_variations::{Datum, Success};
use super::super::result::kinds::SdkResult;
use super::super::result::{Failure, Nullable};
use super::super::vector_query_embedding::outbound_client;
use super::providers::{CustomHandlers, Providers};
use super::{Call, Client, Images, Outcome, Run, RunOutcome};

// Each run builds the production operation over the mock's base, the case's custom providers and one recorder
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let client = outbound_client(env)?;
    let loader = ConfigLoader {
        lookup: env.clone(),
    };
    let settings = loader
        .load("model_list: []")
        .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?
        .sections
        .image_edit_call;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let catalog = BundledCatalog::default().get();
    let known_model_names = BTreeSet::new();
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        let custom_providers: BTreeSet<String> = run
            .custom_providers
            .iter()
            .map(|provider| provider.name.clone())
            .collect();
        let resolution = ResolutionContext {
            catalog: &catalog,
            known_model_names: &known_model_names,
            custom_providers: &custom_providers,
            environment: env,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        let providers = Providers::new(run.recorded, &client);
        let custom = CustomHandlers::new(&run.custom_providers);
        let service = ImageEdits {
            settings: &settings,
            resolution: &resolution,
            providers: &providers,
            custom: &custom,
            global_drop_params: false,
        };
        let edit = edit_call(run, api_base);
        let entry = ImageEditEntry {
            operation: &service,
            logger: &mut Unlogged,
            redactor: &redactor,
        };
        let answer = if run.asynchronous {
            entry.aimage_edit(&edit).await
        } else {
            entry.image_edit(&edit).await
        };
        let outcome = match answer {
            Ok(response) => RunOutcome {
                success: Some(success(response)),
                failure: None,
                sent: providers.take(),
                custom: custom.take(),
            },
            Err(exception) => RunOutcome {
                success: None,
                failure: Some(failure(&exception)),
                sent: None,
                custom: None,
            },
        };
        runs.push(outcome);
    }
    Ok(SdkResult::ImageEditCall {
        value: Box::new(Outcome { runs }),
    })
}

// the pre-call record is compared by the behavior tests, so the oracle's logger only accepts it
struct Unlogged;

impl ImageEditLogger for Unlogged {
    fn update_from_kwargs(&mut self, _record: PreCallRecord) -> Result<(), PrePostError> {
        Ok(())
    }
}

// LiteLLM reads a bytes upload with no name, so the file carries only its content
fn upload(content: &str) -> ImageFile {
    ImageFile {
        filename: None,
        content: content.as_bytes().to_vec(),
        content_type: None,
    }
}

// the Python side adds the mock's base to kwargs the same way, except where a custom handler records them
fn edit_call(run: &Run, api_base: &str) -> ImageEditCall {
    let mut kwargs = run.kwargs.clone();
    if run.custom_providers.is_empty() {
        kwargs.insert(String::from("api_base"), Json::Text(api_base.to_owned()));
    }
    ImageEditCall {
        image: match &run.image {
            None => ImageInput::Absent,
            Some(Images::One(content)) => ImageInput::One(Box::new(upload(content))),
            Some(Images::Many(contents)) => {
                ImageInput::Many(contents.iter().map(String::as_str).map(upload).collect())
            }
        },
        prompt: run.prompt.clone(),
        model: run.model.clone(),
        mask: run.mask.as_deref().map(upload),
        count: run.count,
        quality: run.quality,
        response_format: run.response_format,
        size: run.size.clone(),
        user: run.user.clone(),
        extra_headers: run.extra_headers.clone(),
        extra_query: run.extra_query.clone(),
        extra_body: run.extra_body.clone(),
        timeout: run.timeout,
        custom_llm_provider: run.custom_llm_provider.clone(),
        kwargs,
        client: run.client.map(|client| match client {
            Client::Sync => ImageEditClientKind::Sync,
            Client::Asynchronous => ImageEditClientKind::Asynchronous,
        }),
    }
}

fn success(response: ImageResponse) -> Success {
    Success {
        created: response.created,
        data: response
            .data
            .unwrap_or_default()
            .into_iter()
            .map(|datum| Datum {
                b64_json: datum.b64_json,
                revised_prompt: datum.revised_prompt,
                url: datum.url,
            })
            .collect(),
    }
}

// exception_type's class with the fields an openai.APIError carries
fn failure(exception: &ImageEditException) -> Failure {
    let metadata = exception.class.metadata();
    Failure {
        class: class_name(&exception.class).to_owned(),
        status: metadata.status_code,
        kind: nullable(metadata.error_type),
        param: nullable(metadata.param),
        code: Some(metadata.openai_code),
        message: class_message(&exception.class),
    }
}

fn nullable(wire: WireField) -> Nullable {
    match wire {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}
