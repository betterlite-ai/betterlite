use betterlite_models::decisions::Json;

use super::support::{
    body, catalog, eligible, image, json, mapped, mapped_messages, model_cost, options_json,
};
use crate::canvas_edits::params::SUPPORTED_OPENAI_PARAMS;

const NO_CONFIG: &str = "{}";

const SIZE_ROWS: [(&str, &str); 15] = [
    (r#"{"size": "10x20"}"#, r#"{"width":10,"height":20}"#),
    (r#"{"size": "007x8"}"#, r#"{"width":7,"height":8}"#),
    (r#"{"size": "1_0x2_0"}"#, r#"{"width":10,"height":20}"#),
    (r#"{"size": "_1x2"}"#, NO_CONFIG),
    (r#"{"size": "-3x+4"}"#, r#"{"width":-3,"height":4}"#),
    (r#"{"size": "5x-0"}"#, r#"{"width":5,"height":0}"#),
    (r#"{"size": " 10x 20"}"#, r#"{"width":10,"height":20}"#),
    (r#"{"size": "10x20x30"}"#, NO_CONFIG),
    (r#"{"size": "10xzz"}"#, NO_CONFIG),
    (r#"{"size": "10"}"#, NO_CONFIG),
    (r#"{"size": "x"}"#, NO_CONFIG),
    (r#"{"size": null}"#, NO_CONFIG),
    (
        r#"{"size": "10x20", "width": 5}"#,
        r#"{"width":10,"height":20}"#,
    ),
    (r#"{"n": 2}"#, r#"{"numberOfImages":2}"#),
    (r#"{"n": null}"#, NO_CONFIG),
];

const GENERATION_ROWS: [(&str, Option<&str>); 10] = [
    (r#"{"size": "10x20", "width": 5}"#, Some(r#"{"width":5}"#)),
    (r#"{"size": "10x20", "height": 7}"#, Some(r#"{"height":7}"#)),
    (
        r#"{"size": "10xzz", "numberOfImages": 2}"#,
        Some(r#"{"numberOfImages":2}"#),
    ),
    (
        r#"{"imageGenerationConfig": {"seed": 1, "custom": "x"}, "width": 9}"#,
        Some(r#"{"seed":1,"custom":"x","width":9}"#),
    ),
    (
        r#"{"imageGenerationConfig": {"width": 1}, "width": 9, "seed": 6}"#,
        Some(r#"{"width":9,"seed":6}"#),
    ),
    (
        r#"{"imageGenerationConfig": {"seed": 1}}"#,
        Some(r#"{"seed":1}"#),
    ),
    (r#"{"imageGenerationConfig": [1]}"#, None),
    (r#"{"imageGenerationConfig": {}}"#, None),
    (r#"{"imageGenerationConfig": null}"#, None),
    (
        r#"{"width": 1, "height": 2, "numberOfImages": 3, "quality": "hd", "cfgScale": 4.5, "seed": 6}"#,
        Some(r#"{"width":1,"height":2,"numberOfImages":3,"quality":"hd","cfgScale":4.5,"seed":6}"#),
    ),
];

const QUALITY_ROWS: [(&str, &str); 6] = [
    (r#"{"quality": "hd"}"#, r#"{"quality":"premium"}"#),
    (r#"{"quality": "premium"}"#, r#"{"quality":"premium"}"#),
    (r#"{"quality": "standard"}"#, r#"{"quality":"standard"}"#),
    (r#"{"quality": "auto"}"#, r#"{"quality":"auto"}"#),
    (r#"{"quality": null}"#, NO_CONFIG),
    (
        r#"{"n": 2, "size": "1x2"}"#,
        r#"{"numberOfImages":2,"width":1,"height":2}"#,
    ),
];

pub(super) fn size_parse_fallback() {
    size_fallback_rows();
    generation_config_rows();
}

fn size_fallback_rows() {
    for (given, expected) in SIZE_ROWS {
        assert_eq!(mapped(given, false), json(expected), "{given}");
    }
    let source = image("img");
    assert_eq!(
        body(
            Some("cat"),
            Some(&source),
            &options_json(r#"{"size": "10x20"}"#)
        ),
        json(
            r#"{"taskType":"IMAGE_VARIATION","imageVariationParams":{"images":["aW1n"],"text":"cat"},"imageGenerationConfig":{"width":10,"height":20}}"#
        )
    );
}

fn generation_config_rows() {
    let source = image("img");
    for (given, expected) in GENERATION_ROWS {
        let asked = options_json(given);
        let unchanged = options_json(given);
        let answer = body(Some("cat"), Some(&source), &asked);
        let config = match &answer {
            Json::Object(fields) => fields.get("imageGenerationConfig").cloned(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        };
        assert_eq!(config, expected.map(json), "{given}");
        assert_eq!(asked, unchanged, "{given}");
    }
}

pub(super) fn quality_map() {
    let supported: [&str; 13] = [
        "n",
        "size",
        "response_format",
        "mask",
        "negativeText",
        "similarityStrength",
        "cfgScale",
        "seed",
        "quality",
        "taskType",
        "maskPrompt",
        "outPaintingMode",
        "imageGenerationConfig",
    ];
    assert_eq!(SUPPORTED_OPENAI_PARAMS, supported);
    for (given, wanted) in QUALITY_ROWS {
        assert_eq!(mapped(given, false), json(wanted), "{given}");
    }
    assert_eq!(
        mapped(r#"{"user": "u", "_x": 1, "mask": "m"}"#, false),
        json(r#"{"user":"u","_x":1,"mask":"m"}"#)
    );
    assert_eq!(
        mapped(
            r#"{"user": "u", "_x": 1, "mask": "m", "width": 1, "height": 2, "numberOfImages": 3, "taskType": "INPAINTING", "imageGenerationConfig": {"a": 1}, "quality": "hd"}"#,
            true
        ),
        json(
            r#"{"_x":1,"mask":"m","width":1,"height":2,"numberOfImages":3,"taskType":"INPAINTING","imageGenerationConfig":{"a":1},"quality":"premium"}"#
        )
    );
    unknown_options_stay_out_of_body();
}

// Only the named task keys reach the wire body, so a retained custom option changes nothing.
fn unknown_options_stay_out_of_body() {
    assert_eq!(
        body(
            Some("cat"),
            Some(&image("img")),
            &options_json(
                r#"{"user": "u", "width": 1, "taskType": "INPAINTING", "mask": "m", "custom": "x"}"#
            )
        ),
        json(
            r#"{"taskType":"INPAINTING","inPaintingParams":{"image":"aW1n","text":"cat","maskImage":"m"},"imageGenerationConfig":{"width":1}}"#
        )
    );
}

pub(super) fn response_format_warning() {
    assert_eq!(
        mapped_messages(r#"{"response_format": "url"}"#, false),
        ["Nova Canvas image edit ignores response_format=url and returns base64 images"]
    );
    for silent in [
        r#"{"response_format": "b64_json"}"#,
        r#"{"response_format": null}"#,
        "{}",
    ] {
        assert!(mapped_messages(silent, false).is_empty(), "{silent}");
    }
    for given in [
        r#"{"response_format": "url"}"#,
        r#"{"response_format": "b64_json"}"#,
        r#"{"response_format": null}"#,
    ] {
        assert_eq!(mapped(given, false), json(NO_CONFIG), "{given}");
    }
    assert_eq!(
        mapped(r#"{"response_format": "url", "user": "u"}"#, true),
        json(NO_CONFIG)
    );
}

pub(super) fn canvas_catalog_eligibility() {
    let cost = catalog();
    for model in [
        "canvas-fixture-plain",
        "CANVAS-FIXTURE-PLAIN",
        "x/canvas-fixture-suffix",
        "y/z/canvas-fixture-suffix",
        "other/canvas-fixture-bedrock-suffix",
        "us.canvas-fixture-base",
        "bedrock/us.canvas-fixture-base-entry",
        "canvas-fixture-dated-2024-01-01",
        "canvas-fixture-dated",
        "canvas-fixture-mixed",
        "stability.stable-image-inpaint-v1:0",
    ] {
        assert!(eligible(&cost, model), "{model}");
    }
    for model in [
        "",
        "canvas-fixture-absent",
        "canvas-fixture-underside",
        "canvas-fixture-off",
        "canvas-fixture-text",
        "canvas-fixture-null",
        "amazon.nova-canvas-v1:0",
        "bedrock/nova-canvas-v1:0",
    ] {
        assert!(!eligible(&cost, model), "{model}");
    }
    assert!(!eligible(
        &model_cost(r#"{"canvas-fixture-scalar": true}"#),
        "canvas-fixture-scalar"
    ));
    assert!(!eligible(&model_cost(NO_CONFIG), "canvas-fixture-plain"));
    // An empty base model adds no candidate, so the bedrock/ entry must never be reached.
    assert!(!eligible(
        &model_cost(r#"{"bedrock/": {"supports_nova_canvas_image_edit": true}}"#),
        "us."
    ));
}
