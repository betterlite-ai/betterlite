use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::image_contracts::records::ImageParams;
use serde::Serialize;
use serde_json::{from_str, from_value, to_value};

use crate::canvas_edits::eligibility::supports_nova_canvas_edit;
use crate::canvas_edits::error::CanvasEditError;
use crate::canvas_edits::params::{IgnoredResponseFormat, map_openai_params};
use crate::canvas_edits::request::{CanvasEditRequestInput, transform_image_edit_request};
use crate::canvas_edits::selection::{ImageEditConfig, image_edit_config_for_model};
use crate::canvas_edits::types::{
    CanvasEditOptions, CanvasEditRequest, CanvasEditValue, CanvasImageInput,
};
use crate::model_routes::ParamMap;

pub(super) const CANVAS_MODEL: &str = "canvas-fixture-plain";
pub(super) const IN_PAINT: &str = "stability.stable-image-inpaint-v1:0";

// The fixture map the oracle case sends too: one entry per candidate name the eligibility scan builds.
pub(super) const CATALOG: &str = r#"{
    "canvas-fixture-plain": {"litellm_provider": "amazon", "supports_nova_canvas_image_edit": true},
    "canvas-fixture-suffix": {"litellm_provider": "amazon", "supports_nova_canvas_image_edit": true},
    "bedrock/canvas-fixture-bedrock-suffix": {"litellm_provider": "bedrock", "supports_nova_canvas_image_edit": true},
    "canvas-fixture-base": {"litellm_provider": "amazon", "supports_nova_canvas_image_edit": true},
    "bedrock/canvas-fixture-base-entry": {"litellm_provider": "bedrock", "supports_nova_canvas_image_edit": true},
    "bedrock/canvas-fixture-underside": {"litellm_provider": "bedrock", "supports_nova_canvas_image_edit": true},
    "canvas-fixture-dated": {"litellm_provider": "amazon", "supports_nova_canvas_image_edit": true},
    "Canvas-Fixture-Mixed": {"litellm_provider": "amazon", "supports_nova_canvas_image_edit": true},
    "canvas-fixture-off": {"litellm_provider": "amazon", "supports_nova_canvas_image_edit": false},
    "canvas-fixture-text": {"litellm_provider": "amazon", "supports_nova_canvas_image_edit": "true"},
    "canvas-fixture-null": null,
    "stability.stable-image-inpaint-v1:0": {"litellm_provider": "bedrock", "supports_nova_canvas_image_edit": true}
}"#;

pub(super) fn json(source: &str) -> Json {
    from_str(source).expect("the fixture is JSON")
}

pub(super) fn wire<T: Serialize>(value: &T) -> Json {
    from_value(to_value(value).expect("the value serializes")).expect("the value is JSON")
}

pub(super) fn options_json(source: &str) -> CanvasEditOptions {
    let params: ImageParams = from_str(source).expect("the options are JSON");
    CanvasEditOptions::from_params(&params)
}

pub(super) fn with_mask_bytes(mut options: CanvasEditOptions, content: &[u8]) -> CanvasEditOptions {
    options.mask = Some(CanvasEditValue::Bytes(content.to_vec()));
    options
}

pub(super) fn with_mask_tuple(mut options: CanvasEditOptions) -> CanvasEditOptions {
    options.mask = Some(CanvasEditValue::Tuple);
    options
}

pub(super) fn image(content: &str) -> CanvasImageInput {
    CanvasImageInput::Bytes(content.as_bytes().to_vec())
}

pub(super) type Prompt<'prompt> = Option<&'prompt str>;
pub(super) type Source<'source> = Option<&'source CanvasImageInput>;
pub(super) type Edits<'edits> = &'edits CanvasEditOptions;

pub(super) fn prepared(
    prompt: Prompt<'_>,
    image: Source<'_>,
    edits: Edits<'_>,
) -> Result<CanvasEditRequest, CanvasEditError> {
    transform_image_edit_request(&CanvasEditRequestInput {
        prompt,
        image,
        options: edits,
    })
}

pub(super) fn body(prompt: Prompt<'_>, image: Source<'_>, edits: Edits<'_>) -> Json {
    match prepared(prompt, image, edits) {
        Ok(request) => wire(&request.body),
        Err(error) => panic!("the request transforms: {error}"),
    }
}

pub(super) fn refused(prompt: Prompt<'_>, image: Source<'_>, edits: Edits<'_>) -> CanvasEditError {
    match prepared(prompt, image, edits) {
        Err(error) => error,
        Ok(request) => panic!("the request is accepted: {request:?}"),
    }
}

pub(super) fn mapped(source: &str, drop_params: bool) -> Json {
    wire(&map_openai_params(&options_json(source), drop_params).options)
}

pub(super) fn mapped_messages(source: &str, drop_params: bool) -> Vec<String> {
    map_openai_params(&options_json(source), drop_params)
        .ignored_response_format
        .iter()
        .map(IgnoredResponseFormat::message)
        .collect()
}

pub(super) fn model_cost(source: &str) -> ParamMap {
    from_str(source).expect("the model cost fixture is JSON")
}

pub(super) fn catalog() -> ParamMap {
    model_cost(CATALOG)
}

pub(super) fn eligible(model_cost: &ParamMap, model: &str) -> bool {
    let fixtures = Fixtures::new();
    let context = fixtures.context();
    supports_nova_canvas_edit(model, model_cost, &context, &mut ModelCostKeys::default())
}

pub(super) fn config_for(
    model_cost: &ParamMap,
    model: &str,
) -> Result<ImageEditConfig, CanvasEditError> {
    let fixtures = Fixtures::new();
    let context = fixtures.context();
    image_edit_config_for_model(model, model_cost, &context, &mut ModelCostKeys::default())
}

struct Fixtures {
    typed: ModelCatalog,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Fixtures {
    fn new() -> Self {
        Self {
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)
                .expect("an empty catalog parses"),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.typed,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}
