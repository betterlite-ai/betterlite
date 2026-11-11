use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;

use super::support::{
    JPEG_BASE64, MapFetcher, PNG_BASE64, bmp_data_url, jpeg_data_url, messages, options,
    png_data_url, text_message,
};
use crate::ollama_generate::request::{
    OllamaGenerateRequest, RequestInput, RequestParams, async_transform_request, transform_request,
};

const CUSTOM: &str = r#"{"llama3":{"roles":{"user":{"pre_message":"<u>","post_message":"</u>"},"assistant":{"pre_message":"<a>","post_message":"</a>"}},"initial_prompt_value":"<s>","final_prompt_value":"</s>"}}"#;
const REQUEST_CUSTOM: &str =
    r#"{"llama3":{"roles":{"user":{"pre_message":"[","post_message":"]"}}}}"#;
const OTHER_CUSTOM: &str = r#"{"other":{"roles":{}}}"#;

fn prompts(json: &str) -> BTreeMap<String, CustomPromptTemplate> {
    serde_json::from_str(json).unwrap()
}

fn params(json: &str) -> RequestParams {
    serde_json::from_str(json).unwrap()
}

fn body(
    model: &str,
    messages: &[Message],
    optional: &IndexMap<String, Json>,
    request_params: &RequestParams,
    custom: &BTreeMap<String, CustomPromptTemplate>,
) -> OllamaGenerateRequest {
    transform_request(&RequestInput {
        model,
        messages,
        optional_params: optional,
        params: request_params,
        custom_prompts: custom,
    })
    .unwrap()
}

fn image_message(url: &str) -> Message {
    let json = r#"{"role":"user","content":[{"type":"text","text":"see"},{"type":"image_url","image_url":{"url":"URL"}}]}"#;
    messages(&format!("[{}]", json.replace("URL", url))).remove(0)
}

#[test]
fn plain_messages_render_through_the_ollama_template() {
    let rendered = body(
        "llama3",
        &[text_message("user", "hi")],
        &IndexMap::new(),
        &RequestParams::default(),
        &BTreeMap::new(),
    );
    assert_eq!(rendered.model, "llama3");
    assert_eq!(rendered.prompt, "### User:\nhi\n\n");
    assert_eq!(rendered.images, Some(Vec::new()));
    assert_eq!(rendered.options, IndexMap::new());
    assert!(!rendered.stream);
    assert_eq!(rendered.format, None);
    assert_eq!(rendered.think, None);
}

#[test]
fn system_messages_render_before_the_user_turn() {
    let rendered = body(
        "llama3",
        &[text_message("system", "s"), text_message("user", "u")],
        &IndexMap::new(),
        &RequestParams::default(),
        &BTreeMap::new(),
    );
    assert_eq!(rendered.prompt, "### System:\ns\n\n### User:\nu\n\n");
}

#[test]
fn empty_messages_render_an_empty_prompt() {
    let rendered = body(
        "llama3",
        &[],
        &IndexMap::new(),
        &RequestParams::default(),
        &BTreeMap::new(),
    );
    assert_eq!(rendered.prompt, "");
    assert_eq!(rendered.images, Some(Vec::new()));
}

#[test]
fn a_bad_role_is_a_client_error() {
    let error = transform_request(&RequestInput {
        model: "llama3",
        messages: &[text_message("other", "x")],
        optional_params: &IndexMap::new(),
        params: &RequestParams::default(),
        custom_prompts: &BTreeMap::new(),
    })
    .unwrap_err();
    assert_eq!(error.status(), Some(400));
}

#[test]
fn a_global_custom_prompt_replaces_the_renderer() {
    let custom = prompts(CUSTOM);
    let rendered = body(
        "llama3",
        &[text_message("user", "hi")],
        &IndexMap::new(),
        &RequestParams::default(),
        &custom,
    );
    assert_eq!(rendered.prompt, "<s><u>hi</u></s>");
    assert_eq!(rendered.images, None);
}

#[test]
fn a_request_custom_prompt_wins_over_the_global_one() {
    let custom = prompts(CUSTOM);
    let rendered = body(
        "llama3",
        &[text_message("user", "hi")],
        &IndexMap::new(),
        &params(&format!(r#"{{"custom_prompt_dict":{REQUEST_CUSTOM}}}"#)),
        &custom,
    );
    assert_eq!(rendered.prompt, "[hi]");
}

#[test]
fn a_request_custom_prompt_dict_replaces_the_global_one() {
    let custom = prompts(CUSTOM);
    let rendered = body(
        "llama3",
        &[text_message("user", "hi")],
        &IndexMap::new(),
        &params(&format!(r#"{{"custom_prompt_dict":{OTHER_CUSTOM}}}"#)),
        &custom,
    );
    assert_eq!(rendered.prompt, "### User:\nhi\n\n");
}

#[test]
fn an_empty_request_custom_prompt_dict_does_not_fall_back() {
    let custom = prompts(CUSTOM);
    let rendered = body(
        "llama3",
        &[text_message("user", "hi")],
        &IndexMap::new(),
        &params(r#"{"custom_prompt_dict":{}}"#),
        &custom,
    );
    assert_eq!(rendered.prompt, "### User:\nhi\n\n");
}

#[test]
fn a_null_request_custom_prompt_dict_falls_back_to_the_global_one() {
    let custom = prompts(CUSTOM);
    let rendered = body(
        "llama3",
        &[text_message("user", "hi")],
        &IndexMap::new(),
        &params(r#"{"custom_prompt_dict":null}"#),
        &custom,
    );
    assert_eq!(rendered.prompt, "<s><u>hi</u></s>");
}

#[test]
fn a_text_completion_call_joins_the_message_text() {
    let rendered = body(
        "llama3",
        &[text_message("user", "hi"), text_message("assistant", "yo")],
        &IndexMap::new(),
        &params(r#"{"text_completion":true}"#),
        &BTreeMap::new(),
    );
    assert_eq!(rendered.prompt, "hiyo");
    assert_eq!(rendered.images, None);
}

#[test]
fn stream_format_and_think_are_taken_from_the_options() {
    let rendered = body(
        "llama3",
        &[text_message("user", "hi")],
        &options(r#"{"stream":true,"format":{"type":"object"},"think":true,"temperature":0.5}"#),
        &RequestParams::default(),
        &BTreeMap::new(),
    );
    assert!(rendered.stream);
    assert_eq!(
        rendered.format,
        Some(Json::Object(options(r#"{"type":"object"}"#)))
    );
    assert_eq!(rendered.think, Some(Json::Bool(true)));
    assert_eq!(
        rendered.options.get("temperature"),
        Some(&Json::Number(serde_json::Number::from_f64(0.5).unwrap()))
    );
}

#[test]
fn null_format_and_think_are_dropped() {
    let rendered = body(
        "llama3",
        &[text_message("user", "hi")],
        &options(r#"{"format":null,"think":null,"stream":null}"#),
        &RequestParams::default(),
        &BTreeMap::new(),
    );
    assert!(!rendered.stream);
    assert_eq!(rendered.format, None);
    assert_eq!(rendered.think, None);
}

#[test]
fn a_non_bool_stream_is_rejected() {
    let error = transform_request(&RequestInput {
        model: "llama3",
        messages: &[text_message("user", "hi")],
        optional_params: &options(r#"{"stream":1}"#),
        params: &RequestParams::default(),
        custom_prompts: &BTreeMap::new(),
    })
    .unwrap_err();
    assert_eq!(error.status(), Some(400));
}

#[test]
fn images_from_the_message_content_are_converted() {
    let rendered = body(
        "llama3",
        &[image_message(&png_data_url())],
        &IndexMap::new(),
        &RequestParams::default(),
        &BTreeMap::new(),
    );
    assert_eq!(rendered.prompt, "### User:\nsee\n\n");
    assert_eq!(rendered.images, Some(vec![String::from(PNG_BASE64)]));
}

#[test]
fn a_jpeg_image_is_returned_as_its_base64() {
    let rendered = body(
        "llama3",
        &[image_message(&jpeg_data_url())],
        &IndexMap::new(),
        &RequestParams::default(),
        &BTreeMap::new(),
    );
    assert_eq!(rendered.images, Some(vec![String::from(JPEG_BASE64)]));
}

#[test]
fn a_bmp_image_is_reencoded_as_jpeg() {
    let rendered = body(
        "llama3",
        &[image_message(&bmp_data_url())],
        &IndexMap::new(),
        &RequestParams::default(),
        &BTreeMap::new(),
    );
    let images = rendered.images.unwrap();
    assert_eq!(images.len(), 1_usize);
    let decoded = STANDARD.decode(&images[0]).unwrap();
    assert!(decoded.starts_with(&[0xFF, 0xD8, 0xFF]));
}

#[test]
fn a_remote_image_url_fails_without_the_async_path() {
    let error = transform_request(&RequestInput {
        model: "llama3",
        messages: &[image_message("http://h/x.png")],
        optional_params: &IndexMap::new(),
        params: &RequestParams::default(),
        custom_prompts: &BTreeMap::new(),
    })
    .unwrap_err();
    assert_eq!(error.status(), Some(400));
}

#[test]
fn the_images_option_is_converted_in_the_custom_prompt_branch() {
    let custom = prompts(CUSTOM);
    let rendered = body(
        "llama3",
        &[text_message("user", "hi")],
        &options(&format!(r#"{{"images":["{}"]}}"#, jpeg_data_url())),
        &RequestParams::default(),
        &custom,
    );
    assert_eq!(rendered.images, Some(vec![String::from(JPEG_BASE64)]));
}

#[test]
fn a_non_list_images_option_is_rejected() {
    let custom = prompts(CUSTOM);
    let error = transform_request(&RequestInput {
        model: "llama3",
        messages: &[text_message("user", "hi")],
        optional_params: &options(r#"{"images":"x"}"#),
        params: &RequestParams::default(),
        custom_prompts: &custom,
    })
    .unwrap_err();
    assert_eq!(error.status(), Some(400));
}

#[test]
fn a_non_text_image_is_rejected() {
    let custom = prompts(CUSTOM);
    let error = transform_request(&RequestInput {
        model: "llama3",
        messages: &[text_message("user", "hi")],
        optional_params: &options(r#"{"images":[5]}"#),
        params: &RequestParams::default(),
        custom_prompts: &custom,
    })
    .unwrap_err();
    assert_eq!(error.status(), Some(400));
}

#[test]
fn a_bad_data_url_is_rejected() {
    let error = transform_request(&RequestInput {
        model: "llama3",
        messages: &[image_message("data:image/png;base64")],
        optional_params: &IndexMap::new(),
        params: &RequestParams::default(),
        custom_prompts: &BTreeMap::new(),
    })
    .unwrap_err();
    assert_eq!(error.status(), Some(400));
}

#[tokio::test]
async fn the_async_path_inlines_remote_images() {
    let fetched = image_message("http://h/x.png");
    let inlined = async_transform_request(
        &RequestInput {
            model: "llama3",
            messages: &[fetched],
            optional_params: &IndexMap::new(),
            params: &RequestParams::default(),
            custom_prompts: &BTreeMap::new(),
        },
        &MapFetcher,
    )
    .await
    .unwrap();
    assert_eq!(inlined.images, Some(vec![String::from(PNG_BASE64)]));
}

#[tokio::test]
async fn the_async_path_reports_a_rejected_fetch() {
    let error = async_transform_request(
        &RequestInput {
            model: "llama3",
            messages: &[image_message("http://h/x.gif")],
            optional_params: &IndexMap::new(),
            params: &RequestParams::default(),
            custom_prompts: &BTreeMap::new(),
        },
        &MapFetcher,
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), Some(400));
}

#[test]
fn a_request_text_completion_that_is_not_a_bool_is_rejected() {
    let mut deserializer = serde_json::Deserializer::from_str(r#"{"text_completion":"yes"}"#);
    let error =
        serde_path_to_error::deserialize::<_, RequestParams>(&mut deserializer).unwrap_err();
    assert!(error.path().to_string().contains("text_completion"));
}

#[test]
fn a_request_message_with_tool_calls_renders_them() {
    let json = r#"{"role":"assistant","content":"x","tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{\"a\":1}"}}]}"#;
    let rendered = body(
        "llama3",
        &messages(&format!("[{json}]")),
        &IndexMap::new(),
        &RequestParams::default(),
        &BTreeMap::new(),
    );
    assert_eq!(
        rendered.prompt,
        "### Assistant:\nxTool Calls: [\n  {\n    \"id\": \"c1\",\n    \"type\": \"function\",\n    \"function\": {\n      \"name\": \"f\",\n      \"arguments\": {\n        \"a\": 1\n      }\n    }\n  }\n]\n\n"
    );
}
