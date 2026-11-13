use crate::identity::provider_id::ProviderId;

// LiteLLM's hardcoded model sets from litellm/constants.py and litellm/__init__.py at the pin.
pub(super) const REPLICATE_ID_LENGTH: usize = 64;

pub(super) const OPENAI_IMAGE_GENERATION_MODELS: [&str; 2] = ["dall-e-2", "dall-e-3"];
pub(super) const OPENAI_VIDEO_GENERATION_MODELS: [&str; 1] = ["sora-2"];

pub(super) const COHERE_EMBEDDING_MODELS: [&str; 7] = [
    "embed-v4.0",
    "embed-english-v3.0",
    "embed-english-light-v3.0",
    "embed-multilingual-v3.0",
    "embed-english-v2.0",
    "embed-english-light-v2.0",
    "embed-multilingual-v2.0",
];

pub(super) const REPLICATE_MODELS: [&str; 8] = [
    "replicate/llama-2-70b-chat:2796ee9483c3fd7aa2e171d38f4ca12251a30609463dcfd4cd76703f22e96cdf",
    "a16z-infra/llama-2-13b-chat:2a7f981751ec7fdf87b5b91ad4db53683a98082e9ff7bfd12c8cd5ea85980a52",
    "meta/codellama-13b:1c914d844307b0588599b8393480a3ba917b660c7e9dfae681542b5325f228db",
    "replicate/vicuna-13b:6282abe6a492de4145d7bb601023762212f9ddbbe78278bd6771c8b3b2f2a13b",
    "joehoover/instructblip-vicuna13b:c4c54e3c8c97cd50c2d2fec9be3b6065563ccf7d43787fb99f84151b867178fe",
    "daanelson/flan-t5-large:ce962b3f6792a57074a601d3979db5839697add2e4e02696b3ced4c022d4767f",
    "replicate/dolly-v2-12b:ef0e1aefc61f8e096ebe4db6b2bacc297daf2ef6899f0f7e001ec445893500e5",
    "replit/replit-code-v1-3b:b84f4c074b807211cd75e3e8b1589b6399052125b4c27106e43d47189e8415ad",
];

pub(super) const MARITALK_MODELS: [&str; 1] = ["maritalk"];
pub(super) const PETALS_MODELS: [&str; 1] = ["petals-team/StableBeluga2"];
pub(super) const BASETEN_MODELS: [&str; 3] = ["qvv0xeq", "q841o8w", "31dxrj3"];
pub(super) const EMPOWER_MODELS: [&str; 2] = [
    "empower/empower-functions",
    "empower/empower-functions-small",
];
pub(super) const OPEN_AI_EMBEDDING_MODELS: [&str; 1] = ["text-embedding-ada-002"];

pub(super) const BEDROCK_EMBEDDING_MODELS: [&str; 7] = [
    "amazon.titan-embed-text-v1",
    "amazon.nova-2-multimodal-embeddings-v1:0",
    "cohere.embed-english-v3",
    "cohere.embed-multilingual-v3",
    "cohere.embed-v4:0",
    "twelvelabs.marengo-embed-2-7-v1:0",
    "twelvelabs.marengo-embed-3-0-v1:0",
];

pub(super) const BEDROCK_CONVERSE_MODELS: [&str; 58] = [
    "qwen.qwen3-coder-480b-a35b-v1:0",
    "qwen.qwen3-coder-next",
    "qwen.qwen3-235b-a22b-2507-v1:0",
    "qwen.qwen3-coder-30b-a3b-v1:0",
    "qwen.qwen3-32b-v1:0",
    "deepseek.v3-v1:0",
    "deepseek.v3.2",
    "openai.gpt-oss-20b-1:0",
    "openai.gpt-oss-120b-1:0",
    "anthropic.claude-haiku-4-5-20251001-v1:0",
    "anthropic.claude-sonnet-4-5-20250929-v1:0",
    "anthropic.claude-fable-5-1",
    "anthropic.claude-fable-5",
    "anthropic.claude-sonnet-5",
    "anthropic.claude-opus-5",
    "anthropic.claude-opus-4-8",
    "anthropic.claude-opus-4-7",
    "anthropic.claude-opus-4-6-v1:0",
    "anthropic.claude-opus-4-6-v1",
    "anthropic.claude-sonnet-4-6",
    "anthropic.claude-opus-4-1-20250805-v1:0",
    "anthropic.claude-opus-4-20250514-v1:0",
    "anthropic.claude-sonnet-4-20250514-v1:0",
    "anthropic.claude-3-7-sonnet-20250219-v1:0",
    "anthropic.claude-3-5-haiku-20241022-v1:0",
    "anthropic.claude-3-5-sonnet-20241022-v2:0",
    "anthropic.claude-3-5-sonnet-20240620-v1:0",
    "anthropic.claude-3-opus-20240229-v1:0",
    "anthropic.claude-3-sonnet-20240229-v1:0",
    "anthropic.claude-3-haiku-20240307-v1:0",
    "anthropic.claude-v2",
    "anthropic.claude-v2:1",
    "anthropic.claude-v1",
    "anthropic.claude-instant-v1",
    "ai21.jamba-instruct-v1:0",
    "ai21.jamba-1-5-mini-v1:0",
    "ai21.jamba-1-5-large-v1:0",
    "meta.llama3-70b-instruct-v1:0",
    "meta.llama3-8b-instruct-v1:0",
    "meta.llama3-1-8b-instruct-v1:0",
    "meta.llama3-1-70b-instruct-v1:0",
    "meta.llama3-1-405b-instruct-v1:0",
    "meta.llama3-70b-instruct-v1:0",
    "mistral.mistral-large-2407-v1:0",
    "mistral.mistral-large-2402-v1:0",
    "mistral.mistral-small-2402-v1:0",
    "meta.llama3-2-1b-instruct-v1:0",
    "meta.llama3-2-3b-instruct-v1:0",
    "meta.llama3-2-11b-instruct-v1:0",
    "meta.llama3-2-90b-instruct-v1:0",
    "amazon.nova-lite-v1:0",
    "amazon.nova-2-lite-v1:0",
    "amazon.nova-2-pro-preview-20251202-v1:0",
    "amazon.nova-pro-v1:0",
    "writer.palmyra-x4-v1:0",
    "writer.palmyra-x5-v1:0",
    "minimax.minimax-m2.1",
    "moonshotai.kimi-k2.5",
];

pub(super) const PREFIX_PROVIDERS: [(&str, ProviderId); 11] = [
    ("bytez/", ProviderId::Bytez),
    ("gdc/", ProviderId::Gdc),
    ("lemonade/", ProviderId::Lemonade),
    ("heroku/", ProviderId::Heroku),
    ("cometapi/", ProviderId::Cometapi),
    ("oci/", ProviderId::Oci),
    ("compactifai/", ProviderId::Compactifai),
    ("ovhcloud/", ProviderId::Ovhcloud),
    ("clarifai/", ProviderId::Clarifai),
    ("amazon_nova", ProviderId::AmazonNova),
    ("sap/", ProviderId::SapGenerativeAiHub),
];
