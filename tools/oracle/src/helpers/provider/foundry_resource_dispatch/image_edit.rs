use betterlite_foundry::image_edits::flux2::Flux2EditRequest;
use betterlite_foundry::image_edits::image_source::ImageSource;
use betterlite_foundry::image_edits::mai::MaiEditRequest;
use betterlite_foundry::image_edits::selection::ImageEditVariant;
use betterlite_foundry::image_generation::params::ImageParams;
use betterlite_openai::image_edits::file::ImageEditFile;
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
pub(crate) struct Input {
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

enum Uploads {
    Flux2(Option<Vec<ImageSource>>),
    Mai(Option<Vec<Option<ImageEditFile>>>),
}

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = runtime()?;
    let mut logging = Recorder;
    let variant =
        ImageEditVariant::select(&input.model).map_err(|error| failure("LeafFailure", error))?;
    let mut uploads = uploads_of(input, variant)?;
    let options = options_for(input, &uploads, &runtime, &mut logging)?;
    let (call, name) = form_call(input, &options, &mut uploads);
    form_of(run_call(call, &runtime, &mut logging), name)
}

fn options_for(
    input: &Input,
    uploads: &Uploads,
    runtime: &Runtime,
    logging: &mut Recorder,
) -> Result<ImageParams, Box<HelperError>> {
    match uploads {
        Uploads::Flux2(_) => match run_call(
            Call::Flux2Params {
                options: &input.options,
                model: &input.model,
                drop_params: false,
            },
            runtime,
            logging,
        )? {
            Outcome::Params(params) => Ok(params),
            Outcome::Variant(_) | Outcome::Url(_) | Outcome::Form(_) | Outcome::Reply(_) => {
                Err(failure("Variant", OTHER))
            }
        },
        Uploads::Mai(_) => Ok(input.options.clone()),
    }
}

fn form_call<'call>(
    input: &'call Input,
    options: &'call ImageParams,
    uploads: &'call mut Uploads,
) -> (Call<'call>, &'static str) {
    match uploads {
        Uploads::Flux2(images) => (
            Call::Flux2Form(Flux2EditRequest {
                model: &input.model,
                prompt: input.prompt.as_deref(),
                images: images.as_deref_mut(),
                options,
            }),
            "flux2",
        ),
        Uploads::Mai(files) => (
            Call::MaiForm(MaiEditRequest {
                model: &input.model,
                prompt: input.prompt.as_deref(),
                image: files.as_deref(),
                options,
            }),
            "mai",
        ),
    }
}

fn uploads_of(input: &Input, variant: ImageEditVariant) -> Result<Uploads, Box<HelperError>> {
    let bytes: Option<Vec<Option<Vec<u8>>>> = input.images.as_ref().map(|fields| {
        fields
            .iter()
            .map(|field| {
                field
                    .as_ref()
                    .map(|present| present.text.clone().into_bytes())
            })
            .collect()
    });
    match variant {
        ImageEditVariant::Flux2 => Ok(Uploads::Flux2(bytes.map(|fields| {
            fields
                .into_iter()
                .map(|field| {
                    field.map_or_else(
                        || ImageSource::Unsupported {
                            kind: String::from("NoneType"),
                        },
                        ImageSource::Bytes,
                    )
                })
                .collect()
        }))),
        ImageEditVariant::Mai => Ok(Uploads::Mai(bytes.map(|fields| {
            fields
                .into_iter()
                .map(|field| {
                    field.map(|content| ImageEditFile {
                        content,
                        name: None,
                    })
                })
                .collect()
        }))),
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

fn form_of(outcome: Result<Outcome, Box<HelperError>>, name: &str) -> HelperResult {
    let Outcome::Form(form) = outcome? else {
        return Err(failure("Variant", OTHER));
    };
    let files: Vec<Value> = form
        .files
        .iter()
        .map(|(field, file)| {
            json!({"field": field, "filename": file.filename, "content_type": file.content_type})
        })
        .collect();
    Ok(json!({"kind": name, "data": form.data, "files": files}))
}
