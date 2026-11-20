use betterlite_foundry::image_edits::flux2::Flux2EditRequest;
use betterlite_foundry::image_edits::image_source::ImageSource;
use betterlite_foundry::image_edits::mai::MaiEditRequest;
use betterlite_foundry::image_edits::selection::ImageEditVariant;
use betterlite_foundry::image_generation::params::ImageParams;
use betterlite_openai::image_edits::file::ImageEditFile;
use betterlite_openai::image_edits::request::ImageEditForm;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::foundry_resource_dispatch::image_edit::{Call, Dispatch, Outcome};
use betterlite_provider::foundry_resource_dispatch::{
    FoundryResourceError, FoundryResourceOperation, FoundryResourceResult, dispatch,
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
    match ImageEditVariant::select(&input.model) {
        Ok(ImageEditVariant::Flux2) => flux2_row(input, &runtime, &mut logging),
        Ok(ImageEditVariant::Mai) => mai_row(input, &runtime, &mut logging),
        Ok(ImageEditVariant::Flux1) => Err(failure("Variant", OTHER)),
        Err(error) => Err(failure("LeafFailure", error)),
    }
}

fn flux2_row(input: &Input, runtime: &Runtime, logging: &mut Recorder) -> HelperResult {
    let operation = FoundryResourceOperation::ImageEdit(Box::new(Dispatch {
        logging: &mut *logging,
        call: Call::Flux2Params {
            options: &input.options,
            model: &input.model,
            drop_params: false,
        },
    }));
    let mapped = match runtime.block_on(dispatch(operation)) {
        Ok(FoundryResourceResult::ImageEdit(Outcome::Params(params))) => params,
        Ok(_) => return Err(failure("Variant", OTHER)),
        Err(error) => return Err(failure("LeafFailure", error)),
    };
    let mut images = sources(input);
    let operation = FoundryResourceOperation::ImageEdit(Box::new(Dispatch {
        logging: &mut *logging,
        call: Call::Flux2Form(Flux2EditRequest {
            model: &input.model,
            prompt: input.prompt.as_deref(),
            images: images.as_deref_mut(),
            options: &mapped,
        }),
    }));
    form_row(runtime.block_on(dispatch(operation)), "flux2")
}

fn mai_row(input: &Input, runtime: &Runtime, logging: &mut Recorder) -> HelperResult {
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
    let operation = FoundryResourceOperation::ImageEdit(Box::new(Dispatch {
        logging: &mut *logging,
        call: Call::MaiForm(MaiEditRequest {
            model: &input.model,
            prompt: input.prompt.as_deref(),
            image: images.as_deref(),
            options: &input.options,
        }),
    }));
    form_row(runtime.block_on(dispatch(operation)), "mai")
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

fn form_row(
    outcome: Result<FoundryResourceResult, FoundryResourceError>,
    kind: &str,
) -> HelperResult {
    match outcome {
        Ok(FoundryResourceResult::ImageEdit(Outcome::Form(form))) => Ok(project(kind, &form)),
        Ok(_) => Err(failure("Variant", OTHER)),
        Err(error) => Err(failure("LeafFailure", error)),
    }
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
