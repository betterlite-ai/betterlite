use betterlite_auth::agent_policy::inference;
use serde::Deserialize;

// the kinds are defined with the managed inference route map, so both sides read one enum
pub type InferenceModelKind = inference::InferenceModelKind;

// general_settings defaults, resolved once by the config loader; an absent key stays None
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDefaults {
    #[serde(default)]
    pub completion_model: Option<String>,
    #[serde(default)]
    pub image_generation_model: Option<String>,
    #[serde(default)]
    pub moderation_model: Option<String>,
}

#[must_use]
pub fn resolve_inference_model(
    body_model: Option<&str>,
    defaults: &ModelDefaults,
    cli_model: Option<&str>,
    endpoint_model: Option<&str>,
    kind: InferenceModelKind,
) -> Option<String> {
    let model = match kind {
        InferenceModelKind::ImageGeneration => cli_model
            .or(endpoint_model)
            .or(defaults.image_generation_model.as_deref())
            .or(body_model),
        InferenceModelKind::ImageEdit => defaults
            .completion_model
            .as_deref()
            .or(cli_model)
            .or(endpoint_model)
            .or(defaults.image_generation_model.as_deref())
            .or(body_model),
        InferenceModelKind::Moderation => cli_model
            .or(defaults.moderation_model.as_deref())
            .or(body_model),
        InferenceModelKind::Speech => cli_model.or(body_model),
        InferenceModelKind::Body => body_model,
        InferenceModelKind::Path => endpoint_model,
        InferenceModelKind::Completion => defaults
            .completion_model
            .as_deref()
            .or(cli_model)
            .or(endpoint_model)
            .or(body_model),
    };
    model.map(str::to_owned)
}
