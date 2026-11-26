use super::support::{body, image, json, options_json, prepared, refused, with_bytes, with_tuple};
use crate::canvas_edits::error::CanvasEditError;
use crate::canvas_edits::types::{CanvasEditOptions, CanvasImageInput};

const TASK_ROWS: [(&str, &str); 13] = [
    (
        r#"{"taskType": "BACKGROUND_REMOVAL", "mask": "m", "negativeText": "n", "width": 8}"#,
        r#"{"taskType":"BACKGROUND_REMOVAL","backgroundRemovalParams":{"image":"aW1n"}}"#,
    ),
    (
        r#"{"taskType": "IMAGE_VARIATION", "mask": "m", "maskPrompt": "mp", "negativeText": "n", "similarityStrength": 0.5}"#,
        r#"{"taskType":"IMAGE_VARIATION","imageVariationParams":{"images":["aW1n"],"text":"cat","negativeText":"n","similarityStrength":0.5}}"#,
    ),
    (
        r#"{"taskType": "OUTPAINTING", "mask": "m", "maskPrompt": "mp", "negativeText": "n", "outPaintingMode": "DEFAULT"}"#,
        r#"{"taskType":"OUTPAINTING","outPaintingParams":{"image":"aW1n","text":"cat","maskPrompt":"mp","negativeText":"n","outPaintingMode":"DEFAULT"}}"#,
    ),
    (
        r#"{"taskType": "OUTPAINTING", "mask": "m"}"#,
        r#"{"taskType":"OUTPAINTING","outPaintingParams":{"image":"aW1n","text":"cat","maskImage":"m"}}"#,
    ),
    (
        r#"{"taskType": "INPAINTING", "mask": "m", "maskPrompt": "mp"}"#,
        r#"{"taskType":"INPAINTING","inPaintingParams":{"image":"aW1n","text":"cat","maskPrompt":"mp"}}"#,
    ),
    (
        r#"{"mask": "m"}"#,
        r#"{"taskType":"INPAINTING","inPaintingParams":{"image":"aW1n","text":"cat","maskImage":"m"}}"#,
    ),
    (
        r#"{"maskPrompt": "mp"}"#,
        r#"{"taskType":"INPAINTING","inPaintingParams":{"image":"aW1n","text":"cat","maskPrompt":"mp"}}"#,
    ),
    (
        r#"{"negativeText": "n", "similarityStrength": 0.25}"#,
        r#"{"taskType":"IMAGE_VARIATION","imageVariationParams":{"images":["aW1n"],"text":"cat","negativeText":"n","similarityStrength":0.25}}"#,
    ),
    (
        r#"{"taskType": "   ", "negativeText": "n"}"#,
        r#"{"taskType":"IMAGE_VARIATION","imageVariationParams":{"images":["aW1n"],"text":"cat","negativeText":"n"}}"#,
    ),
    (
        r#"{"taskType": null}"#,
        r#"{"taskType":"IMAGE_VARIATION","imageVariationParams":{"images":["aW1n"],"text":"cat"}}"#,
    ),
    (
        r#"{"taskType": ""}"#,
        r#"{"taskType":"IMAGE_VARIATION","imageVariationParams":{"images":["aW1n"],"text":"cat"}}"#,
    ),
    (
        r#"{"mask": null}"#,
        r#"{"taskType":"IMAGE_VARIATION","imageVariationParams":{"images":["aW1n"],"text":"cat"}}"#,
    ),
    (
        r#"{"taskType": "INPAINTING", "mask": "m", "outPaintingMode": "DEFAULT", "similarityStrength": 0.5}"#,
        r#"{"taskType":"INPAINTING","inPaintingParams":{"image":"aW1n","text":"cat","maskImage":"m"}}"#,
    ),
];

pub(super) fn canvas_task_types() {
    let source = image("img");
    for (given, expected) in TASK_ROWS {
        assert_eq!(
            body(Some("cat"), Some(&source), &options_json(given)),
            json(expected),
            "{given}"
        );
    }
    assert_eq!(
        body(None, Some(&source), &options_json("{}")),
        json(
            r#"{"taskType":"IMAGE_VARIATION","imageVariationParams":{"images":["aW1n"],"text":" "}}"#
        )
    );
    assert_eq!(
        body(
            Some(""),
            Some(&source),
            &options_json(r#"{"taskType": "IMAGE_VARIATION"}"#)
        ),
        json(
            r#"{"taskType":"IMAGE_VARIATION","imageVariationParams":{"images":["aW1n"],"text":" "}}"#
        )
    );
    assert_eq!(
        body(
            Some("  "),
            Some(&source),
            &options_json(r#"{"taskType": "INPAINTING", "mask": "m"}"#)
        ),
        json(
            r#"{"taskType":"INPAINTING","inPaintingParams":{"image":"aW1n","text":"  ","maskImage":"m"}}"#
        )
    );
}

pub(super) fn canvas_task_errors() {
    let source = image("img");
    mask_requirements(&source);
    unsupported_task_types(&source);
    prompt_is_required(&source);
    empty_mask_prompt_is_present();
    wrong_type_names_the_field(&source);
}

// OUTPAINTING and INPAINTING without a mask each name the two ways to supply one.
fn mask_requirements(source: &CanvasImageInput) {
    let error = refused(
        Some("cat"),
        Some(source),
        &options_json(r#"{"taskType": "OUTPAINTING"}"#),
    );
    assert!(matches!(error, CanvasEditError::OutPaintingMask));
    assert_eq!(
        error.to_string(),
        "OUTPAINTING requires either a mask image or a mask prompt. Pass mask=<file> or maskPrompt=<str> in the request."
    );
    let error = refused(
        Some("cat"),
        Some(source),
        &options_json(r#"{"taskType": "INPAINTING"}"#),
    );
    assert!(matches!(error, CanvasEditError::InPaintingMask));
    assert!(
        error
            .to_string()
            .starts_with("Amazon Nova Canvas INPAINTING requires either maskPrompt or maskImage")
    );
}

// Only the four task names pass, compared exactly: padding and casing are refused, not trimmed.
fn unsupported_task_types(source: &CanvasImageInput) {
    let error = refused(
        Some("cat"),
        Some(source),
        &options_json(r#"{"taskType": "STYLE"}"#),
    );
    assert!(
        matches!(&error, CanvasEditError::UnsupportedTaskType { task_type } if task_type == "STYLE")
    );
    assert_eq!(
        error.to_string(),
        "Unsupported Amazon Nova Canvas taskType: 'STYLE'. Use BACKGROUND_REMOVAL, OUTPAINTING, IMAGE_VARIATION, INPAINTING, or omit taskType for automatic routing (mask → INPAINTING, else IMAGE_VARIATION)."
    );
    for padded in [" INPAINTING ", "inpainting", "IMAGE_VARIATION "] {
        let error = refused(
            Some("cat"),
            Some(source),
            &options_json(&format!(r#"{{"taskType": "{padded}"}}"#)),
        );
        assert!(
            matches!(error, CanvasEditError::UnsupportedTaskType { .. }),
            "{padded}"
        );
    }
    let error = refused(
        None,
        Some(source),
        &options_json(r#"{"taskType": "STYLE"}"#),
    );
    assert!(matches!(error, CanvasEditError::UnsupportedTaskType { .. }));
}

// A painted task needs its prompt before the body looks at a mask.
fn prompt_is_required(source: &CanvasImageInput) {
    let unpainted = options_json(r#"{"taskType": "OUTPAINTING"}"#);
    for prompt in [None, Some("")] {
        let error = refused(prompt, Some(source), &unpainted);
        assert!(
            matches!(error, CanvasEditError::MissingPrompt { .. }),
            "{prompt:?}"
        );
    }
}

// An empty maskPrompt still takes the maskPrompt slot, exactly as LiteLLM's is not None checks do.
fn empty_mask_prompt_is_present() {
    let source = image("img");
    assert_eq!(
        body(
            Some("cat"),
            Some(&source),
            &options_json(r#"{"taskType": "INPAINTING", "maskPrompt": ""}"#)
        ),
        json(
            r#"{"taskType":"INPAINTING","inPaintingParams":{"image":"aW1n","text":"cat","maskPrompt":""}}"#
        )
    );
    assert_eq!(
        body(
            Some("cat"),
            Some(&source),
            &options_json(r#"{"taskType": "OUTPAINTING", "maskPrompt": ""}"#)
        ),
        json(
            r#"{"taskType":"OUTPAINTING","outPaintingParams":{"image":"aW1n","text":"cat","maskPrompt":""}}"#
        )
    );
}

// A value the transform does not declare for a field is refused by name, never coerced.
fn wrong_type_names_the_field(source: &CanvasImageInput) {
    let error = refused(
        Some("cat"),
        Some(source),
        &options_json(r#"{"mask": {"x": 1}}"#),
    );
    assert!(matches!(&error, CanvasEditError::Field { field } if field == "mask"));
    assert_eq!(
        error.to_string(),
        "the mask value has a type the Nova Canvas transform does not declare"
    );
    let error = refused(
        Some("cat"),
        Some(source),
        &options_json(r#"{"taskType": 7}"#),
    );
    assert!(matches!(&error, CanvasEditError::Field { field } if field == "taskType"));
    let error = refused(
        Some("cat"),
        Some(source),
        &with_bytes(options_json(r#"{"width": 5}"#), "width", b"5"),
    );
    assert!(matches!(&error, CanvasEditError::Field { field } if field == "width"));
}

pub(super) fn image_bytes_base64() {
    let given = options_json("{}");
    let bytes = CanvasImageInput::Bytes(b"hello".to_vec());
    assert_eq!(
        body(Some("cat"), Some(&bytes), &given),
        json(
            r#"{"taskType":"IMAGE_VARIATION","imageVariationParams":{"images":["aGVsbG8="],"text":"cat"}}"#
        )
    );
    let empty = CanvasImageInput::Bytes(Vec::new());
    assert_eq!(
        body(Some("cat"), Some(&empty), &given),
        json(
            r#"{"taskType":"IMAGE_VARIATION","imageVariationParams":{"images":[""],"text":"cat"}}"#
        )
    );
    for literal in ["/tmp/picture.png", "https://example.test/picture.png"] {
        let text = CanvasImageInput::Text(literal.to_owned());
        assert_eq!(
            body(Some("cat"), Some(&text), &given),
            json(&format!(
                r#"{{"taskType":"IMAGE_VARIATION","imageVariationParams":{{"images":["{literal}"],"text":"cat"}}}}"#
            )),
            "{literal}"
        );
    }
    missing_image_is_refused(&given);
}

// No image is refused first, whatever the options would have raised later.
fn missing_image_is_refused(given: &CanvasEditOptions) {
    let error = refused(Some("cat"), None, given);
    assert!(matches!(error, CanvasEditError::MissingImage));
    assert_eq!(
        error.to_string(),
        "Nova Canvas image edit requires an image input"
    );
    for options in [
        options_json(r#"{"taskType": "STYLE"}"#),
        options_json(r#"{"taskType": "INPAINTING"}"#),
        with_bytes(given.clone(), "mask", b"m"),
    ] {
        let error = refused(None, None, &options);
        assert!(matches!(error, CanvasEditError::MissingImage));
    }
}

pub(super) fn tuple_file_rejection() {
    let given = options_json("{}");
    let source = image("img");
    let tuple = CanvasImageInput::Tuple;
    let error = refused(Some("cat"), Some(&tuple), &given);
    assert!(matches!(error, CanvasEditError::TupleFileType));
    assert_eq!(
        error.to_string(),
        "Nova Canvas image edit does not support tuple FileTypes. Pass a file-like object, bytes, or a base64-encoded string."
    );
    let error = refused(Some("cat"), Some(&source), &with_tuple(given, "mask"));
    assert!(matches!(error, CanvasEditError::TupleFileType));
    let error = refused(
        Some("cat"),
        Some(&tuple),
        &options_json(r#"{"taskType": "STYLE"}"#),
    );
    assert!(matches!(error, CanvasEditError::TupleFileType));
    assert_eq!(
        body(
            Some("cat"),
            Some(&source),
            &options_json(r#"{"mask": "https://example.test/mask.png"}"#)
        ),
        json(
            r#"{"taskType":"INPAINTING","inPaintingParams":{"image":"aW1n","text":"cat","maskImage":"https://example.test/mask.png"}}"#
        )
    );
    assert_eq!(
        body(
            Some("cat"),
            Some(&CanvasImageInput::Text("aGVsbG8=".to_owned())),
            &with_bytes(options_json("{}"), "mask", b"m")
        ),
        json(
            r#"{"taskType":"INPAINTING","inPaintingParams":{"image":"aGVsbG8=","text":"cat","maskImage":"bQ=="}}"#
        )
    );
}

pub(super) fn mask_versus_mask_prompt() {
    mask_conversion_before_task();
    prompt_rules_for_painted_tasks();
}

fn mask_conversion_before_task() {
    let source = image("img");
    let masked = with_bytes(options_json("{}"), "mask", b"m");
    let request = prepared(Some("cat"), Some(&source), &masked).expect("the request transforms");
    assert_eq!(
        request.body,
        json(
            r#"{"taskType":"INPAINTING","inPaintingParams":{"image":"aW1n","text":"cat","maskImage":"bQ=="}}"#
        )
    );
    assert!(request.files.is_empty());
    assert_eq!(masked, with_bytes(options_json("{}"), "mask", b"m"));
    let error = refused(
        Some("cat"),
        Some(&source),
        &with_tuple(options_json(r#"{"taskType": "IMAGE_VARIATION"}"#), "mask"),
    );
    assert!(matches!(error, CanvasEditError::TupleFileType));
}

fn prompt_rules_for_painted_tasks() {
    let source = image("img");
    for task in ["INPAINTING", "OUTPAINTING"] {
        for prompt in [None, Some("")] {
            let given = options_json(&format!(r#"{{"taskType": "{task}", "maskPrompt": "mp"}}"#));
            let error = refused(prompt, Some(&source), &given);
            assert!(
                matches!(&error, CanvasEditError::MissingPrompt { task_type } if task_type == task),
                "{task} {prompt:?}"
            );
            assert_eq!(
                error.to_string(),
                format!(
                    "Amazon Nova Canvas {task} requires a text prompt. Pass a non-empty `prompt` in your request."
                )
            );
        }
    }
    assert_eq!(
        body(
            Some("  "),
            Some(&source),
            &options_json(r#"{"taskType": "OUTPAINTING", "maskPrompt": "mp"}"#)
        ),
        json(
            r#"{"taskType":"OUTPAINTING","outPaintingParams":{"image":"aW1n","text":"  ","maskPrompt":"mp"}}"#
        )
    );
    assert_eq!(
        body(
            Some(""),
            Some(&source),
            &with_bytes(options_json("{}"), "mask", b"m")
        ),
        json(
            r#"{"taskType":"INPAINTING","inPaintingParams":{"image":"aW1n","text":" ","maskImage":"bQ=="}}"#
        )
    );
}
