use betterlite_foundry::image_edits::flux2::Flux2EditRequest;
use betterlite_foundry::image_edits::image_source::ImageSource;
use betterlite_foundry::image_edits::mai::MaiEditRequest;
use betterlite_foundry::image_edits::selection::ImageEditVariant;
use betterlite_foundry::image_generation::params::ImageParams;
use betterlite_openai::image_edits::file::ImageEditFile;
use betterlite_openai::image_edits::request::ImageEditForm;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::foundry_resource_dispatch::image_edit::{Call, Dispatch, Outcome};
use betterlite_provider::foundry_resource_dispatch::{
    FoundryResourceOperation, FoundryResourceResult, dispatch,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::support::{OTHER, Recorder, failure, runtime};

#[derive(Debug, Deserialize)]
pub(super) struct Input {
    model: String,
    prompt: Option<String>,
    images: Option<Vec<Option<ImageField>>>,
    #[serde(default)]
    options: ImageParams,
}

#[derive(Debug, Deserialize)]
pub(super) struct ImageField {
    #[serde(default)]
    text: String,
}

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = runtime()?;
    let mut logging = Recorder;
    edit_row(input, &runtime, &mut logging)
}

fn edit_row(input: &Input, runtime: &Runtime, logging: &mut Recorder) -> HelperResult {
    let variant =
        ImageEditVariant::select(&input.model).map_err(|error| failure("LeafFailure", error))?;
    match variant {
        ImageEditVariant::Flux2 => {
            let mapped = match run_call(
                Call::Flux2Params {
                    options: &input.options,
                    model: &input.model,
                    drop_params: false,
                },
                runtime,
                logging,
            ) {
                Ok(Outcome::Params(params)) => params,
                Ok(_) => return Err(failure("Variant", OTHER)),
                Err(error) => return Err(error),
            };
            let mut images = sources(input);
            form_of(
                run_call(
                    Call::Flux2Form(Flux2EditRequest {
                        model: &input.model,
                        prompt: input.prompt.as_deref(),
                        images: images.as_deref_mut(),
                        options: &mapped,
                    }),
                    runtime,
                    logging,
                ),
                "flux2",
            )
        }
        ImageEditVariant::Mai => {
            let images = input.images.as_ref().map(|fields| {
                fields
                    .iter()
                    .map(|field| {
                        field.as_ref().map(|present| ImageEditFile {
                            content: present.text.clone().into_bytes(),
                            name: None,
                        })
                    })
                    .collect::<Vec<_>>()
            });
            form_of(
                run_call(
                    Call::MaiForm(MaiEditRequest {
                        model: &input.model,
                        prompt: input.prompt.as_deref(),
                        image: images.as_deref(),
                        options: &input.options,
                    }),
                    runtime,
                    logging,
                ),
                "mai",
            )
        }
        ImageEditVariant::Flux1 => Err(failure("Variant", OTHER)),
    }
}

fn run_call(
    call: Call<'_>,
    runtime: &Runtime,
    logging: &mut Recorder,
) -> Result<Outcome, Box<HelperError>> {
    let operation = FoundryResourceOperation::ImageEdit(Box::new(Dispatch {
        logging: &mut *logging,
        call,
    }));
    match runtime.block_on(dispatch(operation)) {
        Ok(FoundryResourceResult::ImageEdit(outcome)) => Ok(outcome),
        Ok(_) => Err(failure("Variant", OTHER)),
        Err(error) => Err(failure("LeafFailure", error)),
    }
}

fn form_of(outcome: Result<Outcome, Box<HelperError>>, kind: &str) -> HelperResult {
    match outcome {
        Ok(Outcome::Form(form)) => Ok(project(kind, &form)),
        Ok(_) => Err(failure("Variant", OTHER)),
        Err(error) => Err(error),
    }
}

fn sources(input: &Input) -> Option<Vec<ImageSource>> {
    let fields = input.images.as_ref()?;
    Some(
        fields
            .iter()
            .map(|field| {
                field.as_ref().map_or_else(
                    || ImageSource::Unsupported {
                        kind: String::from("NoneType"),
                    },
                    |present| ImageSource::Bytes(present.text.clone().into_bytes()),
                )
            })
            .collect(),
    )
}

fn project(kind: &str, form: &ImageEditForm) -> Value {
    let files: Vec<Value> = form
        .files
        .iter()
        .map(|(field, file)| {
            json!({"field": field, "filename": file.filename, "content_type": file.content_type})
        })
        .collect();
    json!({"kind": kind, "data": form.data, "files": files})
}
