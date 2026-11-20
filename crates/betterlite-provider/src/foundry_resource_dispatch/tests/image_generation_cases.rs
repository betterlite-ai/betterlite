use betterlite_foundry::image_generation::error::FoundryImageError as FoundryError;
use betterlite_foundry::image_generation::params::ImageParams;
use betterlite_foundry::mai_images::error::MaiImageError;
use betterlite_foundry::mai_images::reply::MaiImageReply;
use betterlite_foundry::mai_images::url::ImageOperation;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

use super::fixture::{RecordingImageLogging, parse};
use crate::foundry_resource_dispatch::image_generation::DispatchError as ImageError;
use crate::foundry_resource_dispatch::image_generation::{self, Call, Dispatch, Outcome};
use crate::foundry_resource_dispatch::{
    FoundryResourceError, FoundryResourceOperation, FoundryResourceResult, dispatch,
};

async fn run(
    call: Call<'_>,
    logging: &mut RecordingImageLogging,
) -> Result<FoundryResourceResult, FoundryResourceError> {
    dispatch(FoundryResourceOperation::ImageGeneration(Box::new(
        Dispatch { logging, call },
    )))
    .await
}

async fn outcome(call: Call<'_>, logging: &mut RecordingImageLogging) -> Outcome {
    match run(call, logging).await {
        Ok(FoundryResourceResult::ImageGeneration(outcome)) => outcome,
        Ok(_) | Err(_) => panic!("the image generation call returns its own outcome"),
    }
}

async fn leaf_error(
    call: Call<'_>,
    logging: &mut RecordingImageLogging,
) -> image_generation::DispatchError {
    match run(call, logging).await {
        Err(FoundryResourceError::ImageGeneration(error)) => error,
        Ok(_) | Err(_) => panic!("the image generation call fails in its leaf"),
    }
}

async fn class_of(model: &str) -> &'static str {
    let mut logging = RecordingImageLogging::default();
    let Outcome::Config(found) = outcome(Call::Config { model }, &mut logging).await else {
        panic!("the config call names a class");
    };
    found
}

async fn config_rows() {
    assert_eq!(
        class_of("dall-e-2").await,
        "AzureFoundryDallE2ImageGenerationConfig"
    );
    assert_eq!(
        class_of("DALL-E-3").await,
        "AzureFoundryDallE3ImageGenerationConfig"
    );
    assert_eq!(
        class_of("MAI-Image-2.5").await,
        "AzureFoundryMAIImageGenerationConfig"
    );
    assert_eq!(
        class_of("flux.2-pro").await,
        "AzureFoundryFluxImageGenerationConfig"
    );
    assert_eq!(
        class_of("gpt-image-1").await,
        "AzureFoundryGPTImageGenerationConfig"
    );
}

async fn flux2_rows() {
    let mut logging = RecordingImageLogging::default();
    let Outcome::Url(url) = outcome(
        Call::Flux2Url {
            api_base: Some("https://res.test/"),
            model: "FLUX.2-flex",
            api_version: None,
        },
        &mut logging,
    )
    .await
    else {
        panic!("the FLUX 2 url call returns a url");
    };
    assert_eq!(
        url,
        "https://res.test/providers/blackforestlabs/v1/flux-2-flex?api-version=preview"
    );
    let missing = leaf_error(
        Call::Flux2Url {
            api_base: None,
            model: "flux.2-pro",
            api_version: None,
        },
        &mut logging,
    )
    .await;
    assert!(matches!(
        missing,
        ImageError::Image(FoundryError::ApiBaseRequired)
    ));
}

async fn mai_rows() {
    let mut logging = RecordingImageLogging::default();
    let cases = [
        (
            ImageOperation::Generation,
            Some("https://res.test/"),
            Some("2025-01-01"),
            "https://res.test/mai/v1/images/generations?api-version=2025-01-01",
        ),
        (
            ImageOperation::Edit,
            Some("https://res.test"),
            None,
            "https://res.test/mai/v1/images/edits?api-version=preview",
        ),
    ];
    for (operation, api_base, api_version, expected) in cases {
        let Outcome::Url(url) = outcome(
            Call::MaiUrl {
                operation,
                api_base,
                api_version,
            },
            &mut logging,
        )
        .await
        else {
            panic!("the MAI url call returns a url");
        };
        assert_eq!(url, expected);
    }
}

async fn flux_params_of(call: Call<'_>, logging: &mut RecordingImageLogging) -> ImageParams {
    let Outcome::FluxParams(found) = outcome(call, logging).await else {
        panic!("the FLUX params call returns the mapped params");
    };
    found
}

async fn flux2_params_rows() {
    let mut logging = RecordingImageLogging::default();
    let options: ImageParams = parse(r#"{"size":"1024x1024","n":2,"seed":"7","quality":"high"}"#);
    let mapped = flux_params_of(
        Call::Flux2Params {
            non_default: &options,
            optional: &ImageParams::new(),
            model: "flux.2-pro",
            drop_params: false,
        },
        &mut logging,
    )
    .await;
    let expected: ImageParams = parse(r#"{"width":1024,"height":1024,"num_images":2,"seed":7}"#);
    assert_eq!(mapped, expected);
}

async fn flux2_params_reject_rows() {
    let mut logging = RecordingImageLogging::default();
    let unsupported: ImageParams = parse(r#"{"style":"vivid"}"#);
    let error = leaf_error(
        Call::Flux2Params {
            non_default: &unsupported,
            optional: &ImageParams::new(),
            model: "flux.2-pro",
            drop_params: false,
        },
        &mut logging,
    )
    .await;
    assert!(matches!(
        error,
        ImageError::Image(FoundryError::UnsupportedParams { .. })
    ));
    let dropped = flux_params_of(
        Call::Flux2Params {
            non_default: &unsupported,
            optional: &ImageParams::new(),
            model: "flux.2-pro",
            drop_params: true,
        },
        &mut logging,
    )
    .await;
    assert!(dropped.is_empty());
}

async fn mai_params_rows() {
    let mut logging = RecordingImageLogging::default();
    let cases = [
        (r#"{"size":"1024x768"}"#, r#"{"width":1024,"height":768}"#),
        ("{}", r#"{"width":1024,"height":1024}"#),
    ];
    for (request, expected) in cases {
        let non_default: Object = parse(request);
        let Outcome::MaiParams(mapped) = outcome(
            Call::MaiParams {
                non_default: &non_default,
                optional: &Object::new(),
                model: "mai-image",
                drop_params: false,
            },
            &mut logging,
        )
        .await
        else {
            panic!("the MAI params call returns the mapped params");
        };
        let wanted: Object = parse(expected);
        assert_eq!(mapped, wanted);
    }
}

async fn mai_params_reject_rows() {
    let mut logging = RecordingImageLogging::default();
    let above: Object = parse(r#"{"size":"1792x1024"}"#);
    let above_error = leaf_error(
        Call::MaiParams {
            non_default: &above,
            optional: &Object::new(),
            model: "mai-image",
            drop_params: false,
        },
        &mut logging,
    )
    .await;
    assert!(matches!(
        above_error,
        image_generation::DispatchError::Mai(MaiImageError::SizeAboveMaximum { .. })
    ));
    let counts: Object = parse(r#"{"n":2}"#);
    let error = leaf_error(
        Call::MaiParams {
            non_default: &counts,
            optional: &Object::new(),
            model: "mai-image",
            drop_params: false,
        },
        &mut logging,
    )
    .await;
    assert!(matches!(
        error,
        image_generation::DispatchError::Mai(MaiImageError::CountNotOne { .. })
    ));
}

async fn mai_reply_rows() {
    let mut logging = RecordingImageLogging::default();
    let request_data: Object = parse(r#"{"prompt":"draw a cat"}"#);
    let optional: Object = parse(r#"{"width":512,"height":512}"#);
    let key = SecretString::from("sk-mai");
    let body = br#"{"created":1,"data":[{"url":"https://img.test/cat.png"}],"usage":{"input_tokens":2,"output_tokens":3}}"#;
    let Outcome::Reply(reply) = outcome(
        Call::MaiReply(MaiImageReply {
            body: &body[..],
            status_code: 200,
            request_data: &request_data,
            optional_params: &optional,
            api_key: Some(&key),
        }),
        &mut logging,
    )
    .await
    else {
        panic!("the MAI reply call returns its projection");
    };
    assert_eq!(reply.size, "512x512");
    let expected: Object = parse(
        r#"{"created":1,"data":[{"url":"https://img.test/cat.png"}],"usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5,"input_tokens_details":{"image_tokens":0,"text_tokens":2}}}"#,
    );
    assert_eq!(reply.body, expected);
    assert_eq!(logging.inputs, [String::from("draw a cat")]);
}

async fn mai_reply_error_row() {
    let mut logging = RecordingImageLogging::default();
    let empty = Object::new();
    let error = leaf_error(
        Call::MaiReply(MaiImageReply {
            body: b"bad gateway",
            status_code: 502,
            request_data: &empty,
            optional_params: &empty,
            api_key: None,
        }),
        &mut logging,
    )
    .await;
    assert!(matches!(
        error,
        image_generation::DispatchError::Mai(MaiImageError::NotJson { .. })
    ));
}

pub(super) async fn check() {
    config_rows().await;
    flux2_rows().await;
    mai_rows().await;
    flux2_params_rows().await;
    flux2_params_reject_rows().await;
    mai_params_rows().await;
    mai_params_reject_rows().await;
    mai_reply_rows().await;
    mai_reply_error_row().await;
}
