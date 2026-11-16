pub mod create;
pub mod environment;
pub mod error;
pub mod image;
pub mod params;
pub mod url;

mod tests;

// get_supported_openai_params: Studio Veo understands only these OpenAI video options
pub const SUPPORTED_OPENAI_PARAMS: [&str; 5] =
    ["model", "prompt", "input_reference", "seconds", "size"];
