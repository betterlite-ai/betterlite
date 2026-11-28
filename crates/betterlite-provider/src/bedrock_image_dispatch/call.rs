use std::time::{Duration, SystemTime};

use betterlite_bedrock::image_call::plan::ImageEnvironment;
use betterlite_bedrock::image_call::transport::{ClientFactory, SuppliedClient};
use betterlite_bedrock::image_edits::plan::{EditEnvironment, EditImage, EditPlan};
use betterlite_bedrock::image_models::params::Params;
use betterlite_openai::api::image_contracts::records::ImageParams;
use secrecy::SecretString;

// The closed provider enum is the only entry: an operation outside generation and edit is reported, never guessed
pub enum BedrockImageOperation<'call, 'cost, 'context> {
    Generate(GenerateOperation<'call>),
    Edit(EditOperation<'call, 'cost, 'context>),
    Unsupported { operation: &'static str },
}

// The call LiteLLM's generation arm hands its handler; the arm reads extra_headers and forwards neither it nor headers
pub struct GenerateOperation<'call> {
    pub model: Option<&'call str>,
    pub prompt: &'call str,
    pub optional_params: Params,
    pub extra_headers: Option<&'call [(String, String)]>,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub timeout: Option<Duration>,
    pub aimg_generation: bool,
    pub environment: ImageEnvironment<'call>,
    pub now: SystemTime,
    pub created: i64,
}

// The edit arm's call; optional_params is the map after the arm merges the non-default parameters into it
pub struct EditOperation<'call, 'cost, 'context> {
    pub model: Option<&'call str>,
    pub prompt: Option<&'call str>,
    pub images: &'call [EditImage],
    pub optional_params: ImageParams,
    pub extra_headers: Option<&'call [(String, String)]>,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub timeout: Option<Duration>,
    pub aimage_edit: bool,
    pub environment: EditEnvironment<'call>,
    pub now: SystemTime,
    pub plan: EditPlan<'cost, 'context>,
}

// The one transport both leaves send through, so the dispatcher keeps its five arguments
pub struct Transport<'client, Factory>
where
    Factory: ClientFactory,
{
    pub factory: &'client Factory,
    pub supplied: Option<SuppliedClient<'client, Factory::Client>>,
}
