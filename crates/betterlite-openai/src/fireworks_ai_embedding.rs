mod tests;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

const DIMENSIONS_PARAM: &str = "dimensions";
const NOMIC_FAMILY: &str = "nomic-ai";

#[must_use]
pub fn supported_openai_params(model: &str) -> &'static [&'static str] {
    if model.contains(NOMIC_FAMILY) {
        &[DIMENSIONS_PARAM]
    } else {
        &[]
    }
}
#[must_use]
pub fn map_openai_params(
    non_default_params: &IndexMap<String, Json>,
    mut optional_params: IndexMap<String, Json>,
    model: &str,
) -> IndexMap<String, Json> {
    let supported = supported_openai_params(model);
    for (param, value) in non_default_params {
        if supported.contains(&param.as_str()) {
            optional_params.insert(param.clone(), value.clone());
        }
    }
    optional_params
}
