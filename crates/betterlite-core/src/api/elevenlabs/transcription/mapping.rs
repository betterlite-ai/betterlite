use betterlite_openai::openai_base_config::Object;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 2] = ["language", "temperature"];
const LANGUAGE_PARAM: &str = "language";
const LANGUAGE_CODE_PARAM: &str = "language_code";

// LiteLLM maps the OpenAI spelling to ElevenLabs' language_code and copies only the supported names
pub fn map_openai_params(non_default_params: &Object, optional_params: &mut Object) {
    for (name, value) in non_default_params {
        if name.as_str() == LANGUAGE_PARAM {
            optional_params.insert(LANGUAGE_CODE_PARAM.to_owned(), value.clone());
        } else if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}
