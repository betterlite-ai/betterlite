#![cfg(test)]

use rstest::rstest;

use super::error::ImageModelsError;
use super::family::{Family, select_family};
use super::nova;
use super::params::{Field, Params};
use super::response::{
    FinishReason, ImageData, ImagesOnlyResponse, ModelResponse, Stability1Response,
    Stability3Response, TitanResponse, optional_images, stability1_images, titan_images,
};
use super::sizing::SizeError;
use super::stability1::{self, ClassSettings};
use super::stability3;
use super::titan;

fn params(pairs: Vec<(&str, Field)>) -> Params {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

#[rstest]
#[case::titan_image_body("titan_image_body")]
#[case::stability1_text_prompts("stability1_text_prompts")]
#[case::stability3_prompt("stability3_prompt")]
#[case::nova_canvas_task_body("nova_canvas_task_body")]
#[case::size_n_quality_map("size_n_quality_map")]
#[case::artifacts_to_b64_results("artifacts_to_b64_results")]
#[case::image_provider_error("image_provider_error")]
#[case::image_family_selection("image_family_selection")]
fn image_models_cases(#[case] name: &str) {
    match name {
        "titan_image_body" => titan_image_body(),
        "stability1_text_prompts" => stability1_text_prompts(),
        "stability3_prompt" => stability3_prompt(),
        "nova_canvas_task_body" => nova_canvas_task_body(),
        "size_n_quality_map" => size_n_quality_map(),
        "artifacts_to_b64_results" => artifacts_to_b64_results(),
        "image_provider_error" => image_provider_error(),
        "image_family_selection" => image_family_selection(),
        _ => panic!("unknown case {name}"),
    }
}

fn titan_image_body() {
    titan_image_body_maps_and_builds();
    titan_image_body_defaults();
    titan_image_body_quality_noop();
}

fn titan_image_body_maps_and_builds() {
    let non_default = params(vec![
        ("size", Field::from("512x512")),
        ("n", Field::Int(2)),
        ("quality", Field::from("hd")),
    ]);
    let mapped = titan::map_openai_params(&non_default, Params::new()).unwrap();
    let expected_config = params(vec![
        ("width", Field::Int(512)),
        ("height", Field::Int(512)),
        ("numberOfImages", Field::Int(2)),
        ("quality", Field::from("premium")),
    ]);
    assert_eq!(
        mapped.get("imageGenerationConfig"),
        Some(&Field::from(expected_config.clone()))
    );

    let mut request_params = mapped;
    request_params.insert("negativeText".to_owned(), Field::from("blurry"));
    let body = titan::transform_request_body("a cat", request_params).unwrap();
    assert_eq!(
        body,
        params(vec![
            ("taskType", Field::from("TEXT_IMAGE")),
            (
                "textToImageParams",
                Field::from(params(vec![
                    ("text", Field::from("a cat")),
                    ("negativeText", Field::from("blurry")),
                ])),
            ),
            ("imageGenerationConfig", Field::from(expected_config)),
        ])
    );
}

fn titan_image_body_defaults() {
    // A falsy negativeText is dropped, and an explicit taskType=None is retained.
    let request_params = params(vec![
        ("negativeText", Field::from("")),
        ("taskType", Field::Null),
    ]);
    let body = titan::transform_request_body("a dog", request_params).unwrap();
    assert_eq!(body.get("taskType"), Some(&Field::Null));
    assert_eq!(
        body.get("textToImageParams"),
        Some(&Field::from(params(vec![("text", Field::from("a dog"))])))
    );
}

fn titan_image_body_quality_noop() {
    // auto/unknown quality adds nothing, so no imageGenerationConfig override is produced.
    let non_default = params(vec![("quality", Field::from("auto"))]);
    let mapped = titan::map_openai_params(&non_default, Params::new()).unwrap();
    assert!(mapped.get("imageGenerationConfig").is_none());
}

fn stability1_text_prompts() {
    stability1_text_prompts_merges_class_settings();
    stability1_text_prompts_override_wins();
    stability1_text_prompts_size_mapping();
}

fn stability1_text_prompts_merges_class_settings() {
    let optional_params = params(vec![
        ("user", Field::from("someone")),
        ("cfg_scale", Field::Int(5)),
    ]);
    let settings = ClassSettings {
        cfg_scale: Some(Field::Int(7)),
        seed: Some(Field::Int(42)),
        steps: None,
        width: None,
        height: None,
    };

    let body = stability1::transform_request_body("a line\nbreak", &optional_params, &settings);
    assert!(!body.contains_key("user"));
    assert_eq!(
        body.get("text_prompts"),
        Some(&Field::List(vec![Field::from(params(vec![
            ("text", Field::from("a line break")),
            ("weight", Field::Int(1)),
        ]))]))
    );
    // The caller's explicit cfg_scale wins over the persisted class setting.
    assert_eq!(body.get("cfg_scale"), Some(&Field::Int(5)));
    // seed has no caller override, so the class setting fills it in.
    assert_eq!(body.get("seed"), Some(&Field::Int(42)));
    assert!(!body.contains_key("steps"));
}

fn stability1_text_prompts_override_wins() {
    // A supplied text_prompts key overwrites the literal one (later key wins on merge).
    let overridden = params(vec![(
        "text_prompts",
        Field::List(vec![Field::from(params(vec![
            ("text", Field::from("override")),
            ("weight", Field::Int(2)),
        ]))]),
    )]);
    let body =
        stability1::transform_request_body("ignored", &overridden, &ClassSettings::default());
    assert_eq!(body.get("text_prompts"), overridden.get("text_prompts"));
}

fn stability1_text_prompts_size_mapping() {
    // size maps into top-level width/height ints.
    let non_default = params(vec![("size", Field::from("768x512"))]);
    let mapped = stability1::map_openai_params(&non_default, Params::new()).unwrap();
    assert_eq!(mapped.get("width"), Some(&Field::Int(768)));
    assert_eq!(mapped.get("height"), Some(&Field::Int(512)));
}

fn stability3_prompt() {
    stability3_prompt_builds_body_and_rejects_collision();
    stability3_prompt_identity_mapping();
}

fn stability3_prompt_builds_body_and_rejects_collision() {
    let optional_params = params(vec![("aspect_ratio", Field::from("1:1"))]);
    let body = stability3::transform_request_body("a cat", optional_params).unwrap();
    assert_eq!(
        body,
        params(vec![
            ("prompt", Field::from("a cat")),
            ("aspect_ratio", Field::from("1:1"))
        ])
    );
    // An existing "prompt" key collides with the explicit keyword.
    let colliding_params = params(vec![("prompt", Field::from("other"))]);
    let error = stability3::transform_request_body("a cat", colliding_params).unwrap_err();
    assert!(matches!(
        error,
        ImageModelsError::KeywordCollision("prompt")
    ));
}

fn stability3_prompt_identity_mapping() {
    // Stability3 supports no openai params, and an arbitrary key passes through unchanged.
    assert!(stability3::SUPPORTED_OPENAI_PARAMS.is_empty());
    let optional_params = params(vec![("style_preset", Field::from("anime"))]);
    let body = stability3::transform_request_body("a cat", optional_params).unwrap();
    assert_eq!(body.get("style_preset"), Some(&Field::from("anime")));
}

fn nova_canvas_task_body() {
    nova_canvas_task_body_text_image_and_color_guided();
    nova_canvas_task_body_inpainting();
    nova_canvas_task_body_unsupported();
}

fn nova_canvas_task_body_text_image_and_color_guided() {
    // TEXT_IMAGE is the default when taskType is absent.
    let text_image_params = params(vec![(
        "imageGenerationConfig",
        Field::from(params(vec![("width", Field::Int(512))])),
    )]);
    let text_image_body = nova::transform_request_body("a cat", text_image_params).unwrap();
    assert_eq!(
        text_image_body.get("textToImageParams"),
        Some(&Field::from(params(vec![("text", Field::from("a cat"))])))
    );
    assert_eq!(
        text_image_body.get("imageGenerationConfig"),
        Some(&Field::from(params(vec![("width", Field::Int(512))])))
    );

    // COLOR_GUIDED_GENERATION overlays the nested params onto text, letting explicit text win.
    let color_guided_params = params(vec![
        ("taskType", Field::from("COLOR_GUIDED_GENERATION")),
        (
            "imageGenerationConfig",
            Field::from(params(vec![(
                "colorGuidedGenerationParams",
                Field::from(params(vec![("text", Field::from("explicit"))])),
            )])),
        ),
    ]);
    let color_guided_body =
        nova::transform_request_body("ignored", color_guided_params).unwrap();
    assert_eq!(
        color_guided_body.get("colorGuidedGenerationParams"),
        Some(&Field::from(params(vec![("text", Field::from("explicit"))])))
    );
}

fn nova_canvas_task_body_inpainting() {
    // INPAINTING follows the same nested-params shape.
    let optional_params = params(vec![("taskType", Field::from("INPAINTING"))]);
    let body = nova::transform_request_body("a cat", optional_params).unwrap();
    assert_eq!(
        body.get("inpaintingParams"),
        Some(&Field::from(params(vec![("text", Field::from("a cat"))])))
    );
}

fn nova_canvas_task_body_unsupported() {
    // Any other task type raises NotImplementedError; an explicit taskType=None is also unsupported.
    for (task_type, expected_message) in [(Field::from("OUTPAINTING"), "OUTPAINTING"), (Field::Null, "None")] {
        let optional_params = params(vec![("taskType", task_type)]);
        let error = nova::transform_request_body("a cat", optional_params).unwrap_err();
        let ImageModelsError::TaskTypeUnsupported(message) = error else {
            panic!("expected an unsupported task type");
        };
        assert_eq!(message, expected_message);
    }
}

fn size_n_quality_map() {
    size_n_quality_map_basic();
    size_n_quality_map_quality_untouched();
    size_n_quality_map_split_errors();
    size_n_quality_map_python_int_syntax();
}

fn size_n_quality_map_basic() {
    let non_default = params(vec![("size", Field::from("256x128")), ("n", Field::Int(4))]);
    let mapped = nova::map_openai_params(&non_default, Params::new()).unwrap();
    assert_eq!(mapped.get("width"), Some(&Field::Int(256)));
    assert_eq!(mapped.get("height"), Some(&Field::Int(128)));
    assert_eq!(mapped.get("numberOfImages"), Some(&Field::Int(4)));
}

fn size_n_quality_map_quality_untouched() {
    // Unlike Titan, high/medium/low/auto/unknown leave an existing quality untouched.
    let optional_params = params(vec![("quality", Field::from("premium"))]);
    let non_default = params(vec![("quality", Field::from("low"))]);
    let mapped = nova::map_openai_params(&non_default, optional_params).unwrap();
    assert_eq!(mapped.get("quality"), Some(&Field::from("premium")));
}

fn size_n_quality_map_split_errors() {
    // Size parsing preserves Python's split()/int() failures across every family.
    let not_enough = params(vec![("size", Field::from("512"))]);
    let not_enough_error = nova::map_openai_params(&not_enough, Params::new()).unwrap_err();
    assert!(matches!(
        not_enough_error,
        ImageModelsError::Size(SizeError::NotEnough(1))
    ));

    let too_many = params(vec![("size", Field::from("512x512x512"))]);
    let too_many_error = titan::map_openai_params(&too_many, Params::new()).unwrap_err();
    assert!(matches!(
        too_many_error,
        ImageModelsError::Size(SizeError::TooMany)
    ));

    let invalid_int = params(vec![("size", Field::from("512x5a2"))]);
    let invalid_int_error = stability1::map_openai_params(&invalid_int, Params::new()).unwrap_err();
    assert!(matches!(
        invalid_int_error,
        ImageModelsError::Size(SizeError::InvalidInt(_))
    ));
}

fn size_n_quality_map_python_int_syntax() {
    // Python integer syntax (underscores, signs, whitespace) is accepted.
    let non_default = params(vec![("size", Field::from(" +1_024 x -2_048 "))]);
    let mapped = titan::map_openai_params(&non_default, Params::new()).unwrap();
    assert_eq!(
        mapped.get("imageGenerationConfig"),
        Some(&Field::from(params(vec![
            ("width", Field::Int(1024)),
            ("height", Field::Int(-2048)),
        ])))
    );
}

fn artifacts_to_b64_results() {
    artifacts_to_b64_results_titan();
    artifacts_to_b64_results_stability1();
    artifacts_to_b64_results_default_empty();
}

fn artifacts_to_b64_results_titan() {
    let response = TitanResponse {
        images: vec!["abc".to_owned(), "def".to_owned()],
    };
    let data = titan_images(&response);
    assert_eq!(
        data,
        vec![
            ImageData {
                b64_json: "abc".to_owned()
            },
            ImageData {
                b64_json: "def".to_owned()
            },
        ]
    );

    let mut model_response = ModelResponse {
        created: Some(1_700_000_000),
        data: Vec::new(),
    };
    model_response.data = data.clone();
    assert_eq!(model_response.created, Some(1_700_000_000));
    assert_eq!(model_response.data, data);

    // Titan requires the images key.
    let error = serde_json::from_str::<TitanResponse>("{}").unwrap_err();
    assert!(error.to_string().contains("images"));
}

fn artifacts_to_b64_results_stability1() {
    // Stability1 requires artifacts, and each artifact's base64.
    let response: Stability1Response =
        serde_json::from_str(r#"{"artifacts": [{"base64": "xyz"}]}"#).unwrap();
    assert_eq!(
        stability1_images(&response),
        vec![ImageData {
            b64_json: "xyz".to_owned()
        }]
    );

    let missing_artifacts = serde_json::from_str::<Stability1Response>("{}").unwrap_err();
    assert!(missing_artifacts.to_string().contains("artifacts"));

    let missing_base64 =
        serde_json::from_str::<Stability1Response>(r#"{"artifacts": [{}]}"#).unwrap_err();
    assert!(missing_base64.to_string().contains("base64"));
}

fn artifacts_to_b64_results_default_empty() {
    // Stability3 and Nova default an absent images field to an empty list.
    assert_eq!(
        optional_images(&ImagesOnlyResponse::default()),
        Vec::<ImageData>::new()
    );
}

fn image_provider_error() {
    image_provider_error_joins_messages();
    image_provider_error_passes_when_clean();
    image_provider_error_malformed_join_failure();
    image_provider_error_nova_ignores_error_field();
    image_provider_error_nonmapping_config();
}

fn image_provider_error_joins_messages() {
    // Retained finish_reasons raise a 400 BedrockError joined by "; ".
    let response = Stability3Response {
        images: Vec::new(),
        finish_reasons: vec![
            FinishReason::Text("CONTENT_FILTERED".to_owned()),
            FinishReason::Text(String::new()),
            FinishReason::Text("ERROR".to_owned()),
        ],
    };
    let error = stability3::check_finish_reasons(&response).unwrap_err();
    let ImageModelsError::Provider {
        status,
        error: provider_error,
    } = error
    else {
        panic!("expected a provider error");
    };
    assert_eq!(status, 400);
    assert_eq!(provider_error.error.message, "CONTENT_FILTERED; ERROR");
}

fn image_provider_error_passes_when_clean() {
    // Absent, or all-falsey, finish reasons pass.
    stability3::check_finish_reasons(&Stability3Response::default()).unwrap();

    let all_falsey = Stability3Response {
        images: Vec::new(),
        finish_reasons: vec![
            FinishReason::Boolean(false),
            FinishReason::Other(0),
            FinishReason::Text(String::new()),
        ],
    };
    stability3::check_finish_reasons(&all_falsey).unwrap();
}

fn image_provider_error_malformed_join_failure() {
    // A malformed, non-string item that survives the truthiness filter keeps its native join failure.
    let malformed = Stability3Response {
        images: Vec::new(),
        finish_reasons: vec![FinishReason::Other(1)],
    };
    let error = stability3::check_finish_reasons(&malformed).unwrap_err();
    assert!(matches!(error, ImageModelsError::JoinNotAString(0, "int")));
}

fn image_provider_error_nova_ignores_error_field() {
    // Nova ignores any error field entirely and just maps images.
    let response: ImagesOnlyResponse =
        serde_json::from_str(r#"{"error": "ignored", "images": ["a"]}"#).unwrap();
    assert_eq!(
        optional_images(&response),
        vec![ImageData {
            b64_json: "a".to_owned()
        }]
    );
}

fn image_provider_error_nonmapping_config() {
    // A nonmapping request config value fails at the mapping step, before any later constructor.
    let optional_params = params(vec![(
        "imageGenerationConfig",
        Field::from("not a mapping"),
    )]);
    let error = nova::transform_request_body("a cat", optional_params).unwrap_err();
    assert!(matches!(
        error,
        ImageModelsError::NotAMapping("imageGenerationConfig")
    ));
}

fn image_family_selection() {
    assert_eq!(
        select_family(Some("amazon.titan-image-generator-v1")),
        Family::Titan
    );
    assert_eq!(select_family(Some("amazon.nova-canvas-v1:0")), Family::Nova);
    assert_eq!(
        select_family(Some("stability.sd3-large-v1:0")),
        Family::Stability3
    );
    assert_eq!(
        select_family(Some("stability.stable-image-core-v1:0")),
        Family::Stability3
    );
    assert_eq!(
        select_family(Some("stability.stable-diffusion-xl-v1")),
        Family::Stability1
    );
    // Substring checks, not strict prefixes, and falsy model values fall back to Stability1.
    assert_eq!(
        select_family(Some("bedrock/amazon.titan-image-generator-v2")),
        Family::Titan
    );
    assert_eq!(select_family(None), Family::Stability1);
    assert_eq!(select_family(Some("")), Family::Stability1);
    // Titan wins when multiple markers are present.
    assert_eq!(
        select_family(Some("amazon.titan-amazon.nova-canvas-sd3")),
        Family::Titan
    );
}
