use super::error::ImageModelsError;
use super::params::{Field, Params};
use super::sizing::split_size;

/// The persisted `AmazonStabilityConfig` class settings, loaded from `betterlite-config`.
///
/// `__init__` stores exactly these five non-None values; there are no methods or dunders
/// to exclude once they are plain struct fields.
#[derive(Debug, Clone, Default)]
pub struct ClassSettings {
    pub cfg_scale: Option<Field>,
    pub seed: Option<Field>,
    pub steps: Option<Field>,
    pub width: Option<Field>,
    pub height: Option<Field>,
}

impl ClassSettings {
    /// `get_config`: the non-`None` settings, in a fixed field order.
    #[must_use]
    pub fn get_config(&self) -> Params {
        let mut config = Params::new();
        for (key, setting) in [
            ("cfg_scale", &self.cfg_scale),
            ("seed", &self.seed),
            ("steps", &self.steps),
            ("width", &self.width),
            ("height", &self.height),
        ] {
            if let Some(value) = setting {
                config.insert(key.to_owned(), value.clone());
            }
        }
        config
    }
}

#[must_use]
pub const fn get_supported_openai_params() -> &'static [&'static str] {
    &["size"]
}

pub fn map_openai_params(
    non_default_params: &Params,
    mut optional_params: Params,
) -> Result<Params, ImageModelsError> {
    if let Some(size) = non_default_params.get("size").and_then(Field::as_str) {
        let (width, height) = split_size(size)?;
        optional_params.insert("width".to_owned(), Field::Int(width));
        optional_params.insert("height".to_owned(), Field::Int(height));
    }
    Ok(optional_params)
}

#[must_use]
pub fn transform_request_body(
    text: &str,
    optional_params: &Params,
    settings: &ClassSettings,
) -> Params {
    let mut inference_params = optional_params.clone();
    inference_params.shift_remove("user");
    // The gateway always runs on Linux, where os.linesep is "\n".
    let prompt = text.replace('\n', " ");
    for (key, value) in settings.get_config() {
        inference_params.entry(key).or_insert(value);
    }

    let mut text_prompt = Params::new();
    text_prompt.insert("text".to_owned(), Field::from(prompt));
    text_prompt.insert("weight".to_owned(), Field::Int(1));

    let mut body = Params::new();
    body.insert(
        "text_prompts".to_owned(),
        Field::List(vec![Field::from(text_prompt)]),
    );
    body.extend(inference_params);
    body
}
