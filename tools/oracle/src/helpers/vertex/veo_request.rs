mod tests;

use std::sync::Arc;

use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_vertex::vertex_veo_request::create::{CreateInput, create_request};
use betterlite_vertex::vertex_veo_request::edit::{EditInput, PrefetchedOperation, edit_request};
use betterlite_vertex::vertex_veo_request::image::{B1120Mime, ImageInput};
use betterlite_vertex::vertex_veo_request::params::{MapInput, map_openai_params};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    creates: Vec<CreateStep>,
    #[serde(default)]
    edits: Vec<EditStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateStep {
    model: String,
    prompt: String,
    api_base: String,
    #[serde(default)]
    params: Params,
    extra_body: Option<Object>,
}

// VideoCreateOptionalRequestParams reduced to the keys map_openai_params reads
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Params {
    input_reference: Option<Json>,
    image: Option<Json>,
    parameters: Option<Object>,
    resolution: Option<Json>,
    seconds: Option<String>,
    size: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditStep {
    prompt: String,
    api_base: String,
    video_id: String,
    extra_body: Option<Object>,
    prefetched: Option<PrefetchedOperation>,
}

// the bundled catalog is the vertex_ai/model metadata B193 resolves, as litellm's local cost map is for LiteLLM
struct BundledPrices(Arc<ModelCatalog>);

impl BundledPrices {
    fn bundled() -> Self {
        let mut bundled = BundledCatalog::default();
        Self(bundled.get())
    }
}

impl PriceLookup for BundledPrices {
    fn effective_model_info(&self, _model: &str, _provider: Option<&str>) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        let canonical = self.0.lookup.get(key)?;
        let entry = self.0.entries.get(canonical.as_ref())?;
        serde_json::from_value(serde_json::to_value(&entry.prices).ok()?).ok()
    }

    fn effective_priced_model(&self, _model: &str, _provider: Option<&str>) -> Option<PricedModel> {
        None
    }
}

// videos/utils.py drops null options and lays extra_body's options over the request params the mapper reads
fn pipeline_params(step: &CreateStep) -> Params {
    let mut params = step.params.clone();
    let extra = without_nulls(step.extra_body.as_ref());
    for (name, value) in &extra {
        match name.as_str() {
            "input_reference" => params.input_reference = Some(value.clone()),
            "image" => params.image = Some(value.clone()),
            "parameters" => params.parameters = value.as_object().cloned(),
            "resolution" => params.resolution = Some(value.clone()),
            "seconds" => params.seconds = value.as_str().map(str::to_owned),
            "size" => params.size = value.as_str().map(str::to_owned),
            _ => {}
        }
    }
    params
}

fn without_nulls(extra: Option<&Object>) -> Object {
    let mut kept = Object::new();
    let Some(extra) = extra else {
        return kept;
    };
    for (name, value) in extra {
        if !matches!(value, Json::Null) {
            kept.insert(name.clone(), value.clone());
        }
    }
    kept
}

fn create_step(step: &CreateStep, prices: &dyn PriceLookup) -> Result<Value, Box<HelperError>> {
    let params = pipeline_params(step);
    let extra = without_nulls(step.extra_body.as_ref());
    let mapped = map_openai_params(
        MapInput {
            model: &step.model,
            input_reference: params.input_reference.map(ImageInput::Json),
            image: params.image.map(ImageInput::Json),
            parameters: params.parameters,
            resolution: params.resolution,
            seconds: params.seconds,
            size: params.size,
        },
        prices,
    );
    let form = create_request(
        CreateInput {
            prompt: &step.prompt,
            api_base: &step.api_base,
            mapped,
            extra: &extra,
        },
        &B1120Mime,
    )
    .or_value_error()?;
    Ok(json!({ "url": form.url, "body": form.body }))
}

fn edit_step(step: &EditStep) -> Result<Value, Box<HelperError>> {
    let form = edit_request(&EditInput {
        prompt: &step.prompt,
        video_id: &step.video_id,
        api_base: &step.api_base,
        extra_body: step.extra_body.as_ref(),
        prefetched: step.prefetched.as_ref(),
    })
    .or_value_error()?;
    Ok(json!({ "url": form.url, "body": form.body }))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let prices = BundledPrices::bundled();
        let creates = input
            .creates
            .iter()
            .map(|step| create_step(step, &prices))
            .collect::<Result<Vec<_>, _>>()?;
        let edits = input
            .edits
            .iter()
            .map(edit_step)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "creates": creates, "edits": edits }))
    }
}
