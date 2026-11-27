#![cfg(test)]

mod editing;
mod fixture;
mod generation;
mod unsupported_operation;

use rstest::rstest;

#[derive(Debug, Clone, Copy)]
enum Family {
    TitanImage,
    StabilityImage,
    CanvasImage,
    CanvasEdit,
    StabilityEdit,
}

// One row per bedrock-image-dispatch oracle case file, each driving the production dispatcher
#[rstest]
#[case::titan_image(Family::TitanImage)]
#[case::stability_image(Family::StabilityImage)]
#[case::canvas_image(Family::CanvasImage)]
#[case::canvas_edit(Family::CanvasEdit)]
#[case::stability_edit(Family::StabilityEdit)]
#[tokio::test]
async fn bedrock_image_dispatch_cases(#[case] family: Family) {
    match family {
        Family::TitanImage => generation::titan_image().await,
        Family::StabilityImage => generation::stability_image().await,
        Family::CanvasImage => generation::canvas_image().await,
        Family::CanvasEdit => editing::canvas_edit().await,
        Family::StabilityEdit => editing::stability_edit().await,
    }
}
