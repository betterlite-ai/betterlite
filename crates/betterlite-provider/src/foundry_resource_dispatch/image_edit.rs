use betterlite_foundry::image_edits::error::ImageEditError;
use betterlite_foundry::image_edits::flux2::{
    Flux2EditRequest, map_edit_params, transform_request as transform_flux2,
};
use betterlite_foundry::image_edits::mai::{MaiEditRequest, transform_request as transform_mai};
use betterlite_foundry::image_edits::reply::transform_mai_edit_response;
use betterlite_foundry::image_edits::selection::ImageEditVariant;
use betterlite_foundry::image_edits::url::{CompleteUrlRequest, complete_url};
use betterlite_foundry::image_generation::params::ImageParams;
use betterlite_foundry::mai_images::reply::ImageLogging;
use betterlite_openai::image_edits::request::ImageEditForm;
use betterlite_openai::image_generation::reply::ImageGenerationReply;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error(transparent)]
    Edit(#[from] ImageEditError),
}

// B555's three edit configs, kept apart so each one's own fields and errors reach the leaf
pub struct Dispatch<'call> {
    pub logging: &'call mut dyn ImageLogging,
    pub call: Call<'call>,
}

pub enum Call<'call> {
    Select {
        model: &'call str,
    },
    Url(CompleteUrlRequest<'call>),
    Flux2Params {
        options: &'call ImageParams,
        model: &'call str,
        drop_params: bool,
    },
    Flux2Form(Flux2EditRequest<'call>),
    MaiForm(MaiEditRequest<'call>),
    MaiReply {
        body: &'call [u8],
        status_code: u16,
        created_seconds: i64,
    },
}

pub enum Outcome {
    Variant(ImageEditVariant),
    Url(String),
    Params(ImageParams),
    Form(ImageEditForm),
    Reply(ImageGenerationReply),
}

impl Dispatch<'_> {
    pub fn run(self) -> Result<Outcome, DispatchError> {
        let Self { logging, call } = self;
        Ok(match call {
            Call::Select { model } => Outcome::Variant(ImageEditVariant::select(model)?),
            Call::Url(request) => Outcome::Url(complete_url(&request)?),
            Call::Flux2Params {
                options,
                model,
                drop_params,
            } => Outcome::Params(map_edit_params(options, model, drop_params)?),
            Call::Flux2Form(request) => Outcome::Form(transform_flux2(request)?),
            Call::MaiForm(request) => Outcome::Form(transform_mai(&request)),
            Call::MaiReply {
                body,
                status_code,
                created_seconds,
            } => Outcome::Reply(transform_mai_edit_response(
                body,
                status_code,
                created_seconds,
                logging,
            )?),
        })
    }
}
