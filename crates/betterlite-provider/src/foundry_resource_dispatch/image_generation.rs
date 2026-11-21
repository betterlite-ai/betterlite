use betterlite_foundry::image_generation::config::FoundryImageConfig;
use betterlite_foundry::image_generation::error::FoundryImageError;
use betterlite_foundry::image_generation::flux::flux2_image_generation_url;
use betterlite_foundry::image_generation::params::{
    ImageParams, map_openai_params as map_flux_params,
};
use betterlite_foundry::mai_images::error::MaiImageError;
use betterlite_foundry::mai_images::params::map_openai_params as map_mai_params;
use betterlite_foundry::mai_images::reply::{
    ImageLogging, MaiImageReply, MaiImageResponse, transform_image_reply,
};
use betterlite_foundry::mai_images::url::{ImageOperation, mai_image_url};
use betterlite_prompts::arguments::json::Object;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error(transparent)]
    Image(#[from] FoundryImageError),
    #[error(transparent)]
    Mai(#[from] MaiImageError),
}

// B553's selected config and B554's MAI request/reply work, routed by one operation
pub struct Dispatch<'call> {
    pub logging: &'call mut dyn ImageLogging,
    pub call: Call<'call>,
}

pub enum Call<'call> {
    Config {
        model: &'call str,
    },
    Flux2Url {
        api_base: Option<&'call str>,
        model: &'call str,
        api_version: Option<&'call str>,
    },
    Flux2Params {
        non_default: &'call ImageParams,
        optional: &'call ImageParams,
        model: &'call str,
        drop_params: bool,
    },
    MaiUrl {
        operation: ImageOperation,
        api_base: Option<&'call str>,
        api_version: Option<&'call str>,
    },
    MaiParams {
        non_default: &'call Object,
        optional: &'call Object,
        model: &'call str,
        drop_params: bool,
    },
    MaiReply(MaiImageReply<'call>),
}

pub enum Outcome {
    Config(&'static str),
    Url(String),
    FluxParams(ImageParams),
    MaiParams(Object),
    Reply(Box<MaiImageResponse>),
}

impl Dispatch<'_> {
    pub fn run(self) -> Result<Outcome, DispatchError> {
        let Self { logging, call } = self;
        Ok(match call {
            Call::Config { model } => {
                Outcome::Config(class_name(FoundryImageConfig::select(model)))
            }
            Call::Flux2Url {
                api_base,
                model,
                api_version,
            } => Outcome::Url(flux2_image_generation_url(api_base, model, api_version)?),
            Call::Flux2Params {
                non_default,
                optional,
                model,
                drop_params,
            } => Outcome::FluxParams(map_flux_params(non_default, optional, model, drop_params)?),
            Call::MaiUrl {
                operation,
                api_base,
                api_version,
            } => Outcome::Url(mai_image_url(operation, api_base, api_version)?),
            Call::MaiParams {
                non_default,
                optional,
                model,
                drop_params,
            } => Outcome::MaiParams(map_mai_params(
                non_default,
                optional.clone(),
                model,
                drop_params,
            )?),
            Call::MaiReply(reply) => {
                Outcome::Reply(Box::new(transform_image_reply(&reply, logging)?))
            }
        })
    }
}

// get_azure_ai_image_generation_config's branch, named the way the oracle reports the LiteLLM class
const fn class_name(config: FoundryImageConfig) -> &'static str {
    match config {
        FoundryImageConfig::DallE2 => "AzureFoundryDallE2ImageGenerationConfig",
        FoundryImageConfig::DallE3 => "AzureFoundryDallE3ImageGenerationConfig",
        FoundryImageConfig::Mai => "AzureFoundryMAIImageGenerationConfig",
        FoundryImageConfig::Flux => "AzureFoundryFluxImageGenerationConfig",
        FoundryImageConfig::GptImage => "AzureFoundryGPTImageGenerationConfig",
    }
}
