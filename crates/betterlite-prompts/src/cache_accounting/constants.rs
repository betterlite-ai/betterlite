pub const MAX_CACHE_CONTROL_BLOCKS: usize = 4;

pub const CACHE_BREAKPOINT_KEYS: [&str; 2] = ["cache_control", "prompt_cache_breakpoint"];

pub const MIN_BREAKPOINT_GPT_VERSION: (u64, u64) = (5, 6);

pub const GPT_VERSION_PATTERN: &str = r"^gpt-(\d+)(?:\.(\d+))?";

pub const BREAKPOINT_BLOCK_TYPES: [&str; 8] = [
    "text",
    "image",
    "image_url",
    "file",
    "input_audio",
    "input_text",
    "input_image",
    "input_file",
];

pub const OPENAI_API_HOST: &str = "api.openai.com";

pub const OPENAI_API_BASE_ENV_VARS: [&str; 2] = ["OPENAI_BASE_URL", "OPENAI_API_BASE"];

// A caller whose message list is not the one that goes upstream, today the Responses layer, sets this so role-targeted points wait for the final list.
pub const CARRY_UNMATCHED_MESSAGE_POINTS: &str = "_litellm_carry_unmatched_cache_control_points";

pub const EXTERNAL_BREAKPOINTS_STAMP: &str = "_litellm_external_breakpoints";

pub const OPENAI_DIALECT_STAMP: &str = "_litellm_openai_dialect";

pub const GATEWAY_CACHE_METADATA_KEY: &str = "litellm_gateway_injected_cache";

// No deployment had been chosen when the injection happened, so it rides in the payload every leg sends and is never a real deployment id.
pub const INJECTED_FOR_EVERY_DEPLOYMENT: &str = "";
