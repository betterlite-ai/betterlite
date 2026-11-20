use betterlite_config::sections::foundry_image_edits::Settings as VersionSettings;
use betterlite_foundry::credentials::model_info::ModelInfoAccessors;
use betterlite_foundry::image_edits::error::ImageEditError;
use betterlite_foundry::image_edits::flux2::Flux2EditRequest;
use betterlite_foundry::image_edits::image_source::ImageSource;
use betterlite_foundry::image_edits::mai::MaiEditRequest;
use betterlite_foundry::image_edits::selection::ImageEditVariant;
use betterlite_foundry::image_edits::url::CompleteUrlRequest;
use betterlite_foundry::image_generation::params::ImageParams;
use betterlite_openai::image_edits::file::ImageEditFile;

use super::fixture::{RecordingImageLogging, parse};
use crate::foundry_resource_dispatch::image_edit::{self, Call, Dispatch, Outcome};
use crate::foundry_resource_dispatch::{
    FoundryResourceError, FoundryResourceOperation, FoundryResourceResult, dispatch,
};

async fn run(
    call: Call<'_>,
    logging: &mut RecordingImageLogging,
) -> Result<FoundryResourceResult, FoundryResourceError> {
    dispatch(FoundryResourceOperation::ImageEdit(Box::new(Dispatch {
        logging,
        call,
    })))
    .await
}

async fn outcome(call: Call<'_>, logging: &mut RecordingImageLogging) -> Outcome {
    match run(call, logging).await {
        Ok(FoundryResourceResult::ImageEdit(outcome)) => outcome,
        Ok(_) | Err(_) => panic!("the image edit call returns its own outcome"),
    }
}

async fn leaf_error(call: Call<'_>, logging: &mut RecordingImageLogging) -> image_edit::Error {
    match run(call, logging).await {
        Err(FoundryResourceError::ImageEdit(error)) => error,
        Ok(_) | Err(_) => panic!("the image edit call fails in its leaf"),
    }
}

async fn variant_of(model: &str) -> ImageEditVariant {
    let mut logging = RecordingImageLogging::default();
    let Outcome::Variant(found) = outcome(Call::Select { model }, &mut logging).await else {
        panic!("the select call names a variant");
    };
    found
}

fn accessors() -> ModelInfoAccessors<'static> {
    ModelInfoAccessors {
        global_api_base: None,
        global_api_key: None,
        environment_api_base: None,
        environment_api_key: None,
    }
}

fn url_request<'data>(
    model: &'data str,
    api_base: Option<&'data str>,
    api_version: Option<&'data str>,
    global_api_version: Option<&'data str>,
    environment: &'data VersionSettings,
) -> CompleteUrlRequest<'data> {
    CompleteUrlRequest {
        model,
        accessors: accessors(),
        api_base,
        api_version,
        global_api_version,
        environment,
    }
}

async fn variant_rows() {
    assert_eq!(variant_of("MAI-Image-2.5").await, ImageEditVariant::Mai);
    assert_eq!(variant_of("flux.2-pro").await, ImageEditVariant::Flux2);
    assert_eq!(variant_of("flux-1.1-pro").await, ImageEditVariant::Flux1);
    assert_eq!(variant_of("").await, ImageEditVariant::Flux1);
    let mut logging = RecordingImageLogging::default();
    let error = leaf_error(
        Call::Select {
            model: "gpt-image-1",
        },
        &mut logging,
    )
    .await;
    assert!(matches!(
        error,
        image_edit::Error::Edit(ImageEditError::UnsupportedModel { .. })
    ));
}

async fn url_rows() {
    let mut logging = RecordingImageLogging::default();
    let empty = VersionSettings { api_version: None };
    let environment = VersionSettings {
        api_version: Some(String::from("2024-11-01")),
    };
    let Outcome::Url(explicit) = outcome(
        Call::Url(url_request(
            "flux-1.1-pro",
            Some("https://res.test"),
            Some("2024-05-01-preview"),
            Some("2024-10-01"),
            &environment,
        )),
        &mut logging,
    )
    .await
    else {
        panic!("the FLUX 1 url call returns a url");
    };
    assert_eq!(
        explicit,
        "https://res.test/openai/deployments/flux-1.1-pro/images/edits?api-version=2024-05-01-preview"
    );
    let Outcome::Url(global) = outcome(
        Call::Url(url_request(
            "flux-1.1-pro",
            Some("https://res.test"),
            None,
            Some("2024-10-01"),
            &environment,
        )),
        &mut logging,
    )
    .await
    else {
        panic!("the FLUX 1 url call returns a url");
    };
    assert_eq!(
        global,
        "https://res.test/openai/deployments/flux-1.1-pro/images/edits?api-version=2024-10-01"
    );
    let Outcome::Url(from_environment) = outcome(
        Call::Url(url_request(
            "flux-1.1-pro",
            Some("https://res.test"),
            None,
            None,
            &environment,
        )),
        &mut logging,
    )
    .await
    else {
        panic!("the FLUX 1 url call returns a url");
    };
    assert_eq!(
        from_environment,
        "https://res.test/openai/deployments/flux-1.1-pro/images/edits?api-version=2024-11-01"
    );
    let missing_version = leaf_error(
        Call::Url(url_request(
            "flux-1.1-pro",
            Some("https://res.test"),
            None,
            None,
            &empty,
        )),
        &mut logging,
    )
    .await;
    assert!(matches!(
        missing_version,
        image_edit::Error::Edit(ImageEditError::ApiVersionRequired)
    ));

    let Outcome::Url(flux2) = outcome(
        Call::Url(url_request(
            "flux.2-pro",
            Some("https://res.test"),
            None,
            Some("2024-10-01"),
            &empty,
        )),
        &mut logging,
    )
    .await
    else {
        panic!("the FLUX 2 url call returns a url");
    };
    assert_eq!(
        flux2,
        "https://res.test/providers/blackforestlabs/v1/flux-2-pro?api-version=2024-10-01"
    );

    let Outcome::Url(mai) = outcome(
        Call::Url(url_request(
            "MAI-Image-2.5",
            Some("https://res.test"),
            None,
            Some("2024-10-01"),
            &environment,
        )),
        &mut logging,
    )
    .await
    else {
        panic!("the MAI url call returns a url");
    };
    assert_eq!(
        mai,
        "https://res.test/mai/v1/images/edits?api-version=2024-11-01"
    );
    let missing_base = leaf_error(
        Call::Url(url_request("flux.2-pro", None, None, None, &empty)),
        &mut logging,
    )
    .await;
    assert!(matches!(
        missing_base,
        image_edit::Error::Edit(ImageEditError::ApiBaseRequired)
    ));
}

async fn flux2_params_rows() {
    let mut logging = RecordingImageLogging::default();
    let options: ImageParams = parse(r#"{"size":"1024x1024","n":3}"#);
    let Outcome::Params(mapped) = outcome(
        Call::Flux2Params {
            options: &options,
            model: "flux.2-pro",
            drop_params: false,
        },
        &mut logging,
    )
    .await
    else {
        panic!("the FLUX 2 params call returns the mapped params");
    };
    let expected: ImageParams = parse(r#"{"width":1024,"height":1024,"num_images":3}"#);
    assert_eq!(mapped, expected);

    let unsupported: ImageParams = parse(r#"{"style":"vivid"}"#);
    let error = leaf_error(
        Call::Flux2Params {
            options: &unsupported,
            model: "flux.2-pro",
            drop_params: false,
        },
        &mut logging,
    )
    .await;
    assert!(matches!(
        error,
        image_edit::Error::Edit(ImageEditError::Params(_))
    ));
}

async fn flux2_form_rows() {
    let mut logging = RecordingImageLogging::default();
    let options: ImageParams = parse(r#"{"steps":4}"#);
    let mut images = [ImageSource::Bytes(vec![1, 2, 3])];
    let Outcome::Form(form) = outcome(
        Call::Flux2Form(Flux2EditRequest {
            model: "flux.2-pro",
            prompt: Some("make it blue"),
            images: Some(&mut images),
            options: &options,
        }),
        &mut logging,
    )
    .await
    else {
        panic!("the FLUX 2 form call returns the form");
    };
    let expected: ImageParams =
        parse(r#"{"prompt":"make it blue","model":"flux.2-pro","input_image":"AQID","steps":4}"#);
    assert_eq!(form.data, expected);
    assert!(form.files.is_empty());

    let mut missing = [ImageSource::Unsupported {
        kind: String::from("URL"),
    }];
    let error = leaf_error(
        Call::Flux2Form(Flux2EditRequest {
            model: "flux.2-pro",
            prompt: None,
            images: Some(&mut missing),
            options: &options,
        }),
        &mut logging,
    )
    .await;
    assert!(matches!(
        error,
        image_edit::Error::Edit(ImageEditError::Flux2PromptRequired)
    ));
}

async fn mai_form_rows() {
    let mut logging = RecordingImageLogging::default();
    let options: ImageParams = parse(r#"{"image":"stale","n":1}"#);
    let upload = ImageEditFile {
        content: vec![1, 2, 3],
        name: Some(String::from("cat.png")),
    };
    let images = [Some(upload), None];
    let Outcome::Form(form) = outcome(
        Call::MaiForm(MaiEditRequest {
            model: "MAI-Image-2.5",
            prompt: Some("make it blue"),
            image: Some(&images),
            options: &options,
        }),
        &mut logging,
    )
    .await
    else {
        panic!("the MAI form call returns the form");
    };
    let expected: ImageParams = parse(r#"{"model":"MAI-Image-2.5","n":1,"prompt":"make it blue"}"#);
    assert_eq!(form.data, expected);
    assert_eq!(form.files.len(), 1);
    assert_eq!(form.files[0].0, "image");

    let no_image = [None, None];
    let Outcome::Form(empty) = outcome(
        Call::MaiForm(MaiEditRequest {
            model: "MAI-Image-2.5",
            prompt: None,
            image: Some(&no_image),
            options: &ImageParams::new(),
        }),
        &mut logging,
    )
    .await
    else {
        panic!("a MAI form without an upload still returns the form");
    };
    assert!(empty.files.is_empty());
    assert!(empty.data.contains_key("model"));
}

async fn mai_reply_rows() {
    let mut logging = RecordingImageLogging::default();
    let body =
        br#"{"created":7,"data":[{"b64_json":"AQID"}],"usage":{"input_tokens":2,"output_tokens":3}}"#;
    let Outcome::Reply(reply) = outcome(
        Call::MaiReply {
            body: &body[..],
            status_code: 200,
            created_seconds: 11,
        },
        &mut logging,
    )
    .await
    else {
        panic!("the MAI reply call returns its projection");
    };
    assert_eq!(reply.created, 7);
    assert_eq!(reply.data.len(), 1);
    assert_eq!(reply.usage.total_tokens, 5);
    assert_eq!(logging.inputs, [String::new()]);

    let error = leaf_error(
        Call::MaiReply {
            body: b"bad gateway",
            status_code: 502,
            created_seconds: 11,
        },
        &mut logging,
    )
    .await;
    assert!(matches!(
        error,
        image_edit::Error::Edit(ImageEditError::Mai(_))
    ));
}

pub(super) async fn check() {
    variant_rows().await;
    url_rows().await;
    flux2_params_rows().await;
    flux2_form_rows().await;
    mai_form_rows().await;
    mai_reply_rows().await;
}
