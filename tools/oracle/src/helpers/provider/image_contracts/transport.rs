use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;

use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_provider::api::image_contracts::edit::BaseImageEditConfig;
use betterlite_provider::api::image_contracts::generation::BaseImageGenerationConfig;
use betterlite_provider::api::image_contracts::shared::{ImageEnvironment, ImageUrlRequest};
use betterlite_provider::api::image_contracts::variation::BaseImageVariationConfig;
use serde_json::{Value, json};

use super::fixtures::{EditFixture, GenerationFixture, VariationFixture};
use super::{MODEL, TransportDefaults};

pub(super) fn transport_defaults(fields: &TransportDefaults) -> Value {
    let edit = EditFixture {
        calls: AtomicUsize::new(0),
        seen: Mutex::new(Vec::new()),
        raise_status: None,
    };
    let params = ImageParams::new();
    let environment = BaseImageGenerationConfig::validate_environment(
        &GenerationFixture,
        ImageEnvironment {
            headers: &fields.headers,
            model: MODEL,
            messages: &[],
            optional_params: &params,
            litellm_params: &params,
            api_key: None,
            api_base: None,
        },
    );
    let finalized =
        edit.finalize_image_edit_request_data(fields.finalize_data.clone(), &fields.finalize_url);
    let urls = transport_urls(&fields.api_bases, &fields.edit_bases, &edit);
    json!({
        "generation_urls": urls.generation,
        "variation_urls": urls.variation,
        "edit_urls": urls.edit,
        "generation_environment": environment,
        "generation_multipart": BaseImageGenerationConfig::use_multipart_form_data(&GenerationFixture),
        "edit_multipart": BaseImageEditConfig::use_multipart_form_data(&edit),
        "finalized": finalized,
    })
}

struct TransportUrls {
    generation: Vec<String>,
    variation: Vec<String>,
    edit: Vec<Option<String>>,
}

fn transport_urls(
    api_bases: &[Option<String>],
    edit_bases: &[Option<String>],
    edit: &EditFixture,
) -> TransportUrls {
    let params = ImageParams::new();
    let mut generation = Vec::with_capacity(api_bases.len());
    let mut variation = Vec::with_capacity(api_bases.len());
    for api_base in api_bases {
        let request = ImageUrlRequest {
            api_base: api_base.as_deref(),
            api_key: None,
            model: MODEL,
            optional_params: &params,
            litellm_params: &params,
            stream: None,
        };
        generation.push(BaseImageGenerationConfig::get_complete_url(
            &GenerationFixture,
            request,
        ));
        variation.push(BaseImageVariationConfig::get_complete_url(
            &VariationFixture,
            request,
        ));
    }
    let edit_urls = edit_bases
        .iter()
        .map(|api_base| {
            BaseImageEditConfig::get_complete_url(edit, MODEL, api_base.as_deref(), &params).ok()
        })
        .collect();
    TransportUrls {
        generation,
        variation,
        edit: edit_urls,
    }
}
