use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::resolution::ResolutionContext;

use super::eligibility::supports_nova_canvas_edit;
use super::error::CanvasEditError;
use crate::model_routes::ParamMap;
use crate::stability_edits::environment::COMPLETE_URL;
use crate::stability_edits::selection::is_stability_edit_model;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageEditConfig {
    Stability,
    NovaCanvas,
}

impl ImageEditConfig {
    // B637 builds its own URL; the Nova Canvas config leaves the URL to the native Bedrock transport.
    pub fn complete_url(self) -> Result<String, CanvasEditError> {
        match self {
            Self::Stability => Ok(COMPLETE_URL.to_owned()),
            Self::NovaCanvas => Err(CanvasEditError::CompleteUrl),
        }
    }
}

// LiteLLM tries Stability first, then the Nova Canvas catalog flag, and raises for anything else.
pub fn image_edit_config_for_model(
    model: &str,
    model_cost: &ParamMap,
    resolution: &ResolutionContext<'_>,
    keys: &mut ModelCostKeys,
) -> Result<ImageEditConfig, CanvasEditError> {
    if is_stability_edit_model(Some(model)) {
        return Ok(ImageEditConfig::Stability);
    }
    if supports_nova_canvas_edit(model, model_cost, resolution, keys) {
        return Ok(ImageEditConfig::NovaCanvas);
    }
    Err(CanvasEditError::UnsupportedModel {
        model: model.to_owned(),
    })
}
