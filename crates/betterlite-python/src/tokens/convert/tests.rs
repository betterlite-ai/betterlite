#![cfg(test)]

use betterlite_tokenizer::count::request::ToolChoice;
use betterlite_tokenizer::image::tokens::Mode;
use secrecy::ExposeSecret;

use super::{
    BoundRequest, DecodeRequest, DimensionsRequest, EncodeRequest, ImageTokensRequest, LoadRequest,
    ModelRequest, ModifiedRequest, ResizeRequest, TileRequest, TokenCountRequest, Wire, read,
};

#[test]
fn tokens_request_fields_name_the_request() {
    assert_eq!(TokenCountRequest::FIELD, "token count request");
    assert_eq!(EncodeRequest::FIELD, "encode request");
    assert_eq!(DecodeRequest::FIELD, "decode request");
    assert_eq!(LoadRequest::FIELD, "load tokenizer request");
    assert_eq!(ImageTokensRequest::FIELD, "image tokens request");
    assert_eq!(TileRequest::FIELD, "image tile request");
    assert_eq!(DimensionsRequest::FIELD, "image dimensions request");
    assert_eq!(ResizeRequest::FIELD, "image resize request");
    assert_eq!(BoundRequest::FIELD, "image token bound request");
    assert_eq!(ModifiedRequest::FIELD, "modified max tokens request");
    assert_eq!(ModelRequest::FIELD, "model request");
}

#[test]
fn tokens_request_token_count_reads_every_argument() {
    let request: TokenCountRequest = read(
        r#"{"model":"gpt-4o","text":"hello","count_response_tokens":true,"tools":[{"type":"function"}],"tool_choice":"auto","custom_tokenizer":7}"#,
    )
    .unwrap();
    assert_eq!(request.model, "gpt-4o");
    assert_eq!(request.text.as_deref(), Some("hello"));
    assert!(request.messages.is_none());
    assert_eq!(request.count_response_tokens, Some(true));
    assert_eq!(request.tools.as_ref().map(Vec::len), Some(1));
    assert!(matches!(
        request.tool_choice.as_ref(),
        Some(ToolChoice::Mode(mode)) if mode.as_str() == "auto"
    ));
    assert_eq!(request.custom_tokenizer, Some(7));
}

#[test]
fn tokens_request_token_count_reads_messages() {
    let request: TokenCountRequest = read(
        r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"},{"role":"assistant","tool_calls":[{"function":{"arguments":"{\"a\":1}"}}]}]}"#,
    )
    .unwrap();
    let messages = request.messages.unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[1].tool_calls.as_ref().map(Vec::len), Some(1));
}

#[test]
fn tokens_request_token_count_defaults_to_no_arguments() {
    let request: TokenCountRequest = read("{}").unwrap();
    assert_eq!(request.model, "");
    assert!(request.text.is_none());
    assert!(request.messages.is_none());
    assert_eq!(request.count_response_tokens, None);
    assert_eq!(request.custom_tokenizer, None);
}

#[test]
fn tokens_request_encode_reads_the_model_text_and_handle() {
    let request: EncodeRequest =
        read(r#"{"model":"llama-3","text":"hi","custom_tokenizer":3}"#).unwrap();
    assert_eq!(request.model, "llama-3");
    assert_eq!(request.text, "hi");
    assert_eq!(request.custom_tokenizer, Some(3));
}

#[test]
fn tokens_request_decode_reads_the_tokens() {
    let request: DecodeRequest = read(r#"{"tokens":[1,2,3]}"#).unwrap();
    assert_eq!(request.tokens, [1, 2, 3]);
    let empty: DecodeRequest = read("{}").unwrap();
    assert!(empty.tokens.is_empty());
    assert!(empty.skip_special_tokens);
    assert_eq!(empty.custom_tokenizer, None);
}

#[test]
fn tokens_request_load_reads_a_pretrained_identifier() {
    let request: LoadRequest =
        read(r#"{"kind":"pretrained","identifier":"Xenova/x","auth_token":"token"}"#).unwrap();
    let LoadRequest::Pretrained {
        identifier,
        revision,
        auth_token,
    } = request
    else {
        panic!("the request must name a repository");
    };
    assert_eq!(identifier, "Xenova/x");
    assert_eq!(revision, "main");
    assert_eq!(auth_token.unwrap().expose_secret(), "token");
}

#[test]
fn tokens_request_load_keeps_an_explicit_revision() {
    let request: LoadRequest =
        read(r#"{"kind":"pretrained","identifier":"Xenova/x","revision":"v1"}"#).unwrap();
    let LoadRequest::Pretrained {
        revision,
        auth_token,
        ..
    } = request
    else {
        panic!("the request must name a repository");
    };
    assert_eq!(revision, "v1");
    assert!(auth_token.is_none());
}

#[test]
fn tokens_request_load_reads_a_serialized_tokenizer() {
    let request: LoadRequest = read(r#"{"kind":"json","json":"{}"}"#).unwrap();
    let LoadRequest::Json { json } = request else {
        panic!("the request must carry the tokenizer text");
    };
    assert_eq!(json, "{}");
}

#[test]
fn tokens_request_load_refuses_another_kind() {
    let result = read::<LoadRequest>(r#"{"kind":"downloaded"}"#);
    let Err(error) = result else {
        panic!("an unknown kind must not parse");
    };
    assert!(error.to_string().starts_with("the load tokenizer request"));
}

#[test]
fn tokens_request_image_tokens_reads_the_mode_and_base() {
    let request: ImageTokensRequest = read(
        r#"{"data":"data:image/png;base64,AA==","mode":"high","base_tokens":50,"use_default_image_token_count":true}"#,
    )
    .unwrap();
    assert_eq!(request.mode, Mode::High);
    assert_eq!(request.base_tokens, Some(50));
    assert!(request.use_default_image_token_count);
    let empty: ImageTokensRequest = read("{}").unwrap();
    assert_eq!(empty.mode, Mode::Auto);
    assert_eq!(empty.base_tokens, None);
    assert!(!empty.use_default_image_token_count);
}

#[test]
fn tokens_request_tile_reads_custom_tile_sizes() {
    let request: TileRequest =
        read(r#"{"resized_width":800,"resized_height":600,"tile_width":100,"tile_height":64}"#)
            .unwrap();
    assert_eq!(request.resized_width, 800);
    assert_eq!(request.resized_height, 600);
    assert_eq!(request.settings().tile_width, 100);
    assert_eq!(request.settings().tile_height, 64);
    let default_height: TileRequest =
        read(r#"{"resized_width":800,"resized_height":600,"tile_width":100}"#).unwrap();
    assert_eq!(default_height.settings().tile_height, 512);
}

#[test]
fn tokens_request_dimensions_reads_the_data() {
    let request: DimensionsRequest = read(r#"{"data":"data:image/gif;base64,AA=="}"#).unwrap();
    assert_eq!(request.data, "data:image/gif;base64,AA==");
}

#[test]
fn tokens_request_resize_reads_both_sides() {
    let request: ResizeRequest = read(r#"{"width":100,"height":200}"#).unwrap();
    assert_eq!(request.width, 100);
    assert_eq!(request.height, 200);
}

#[test]
fn tokens_request_bound_reads_an_optional_base() {
    let request: BoundRequest = read("{}").unwrap();
    assert_eq!(request.base_tokens, None);
    let custom: BoundRequest = read(r#"{"base_tokens":100}"#).unwrap();
    assert_eq!(custom.base_tokens, Some(100));
}

#[test]
fn tokens_request_modified_counts_like_token_counter() {
    let request: ModifiedRequest = read(
        r#"{"model":"gpt-4o","base_model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"user_max_tokens":10,"buffer_perc":0.5,"buffer_num":3.0}"#,
    )
    .unwrap();
    assert_eq!(request.user_max_tokens, Some(10));
    let settings = request.settings();
    assert_eq!(settings.buffer_fraction.to_bits(), 0.5_f64.to_bits());
    assert_eq!(settings.buffer_minimum_tokens.to_bits(), 3.0_f64.to_bits());
    let input = request.input_request();
    assert_eq!(input.model, "gpt-4o");
    assert_eq!(input.count_response_tokens, Some(false));
    assert!(input.messages.is_some());
    assert!(input.text.is_none());
    assert!(input.tools.is_none());
    assert!(input.tool_choice.is_none());
    assert!(input.custom_tokenizer.is_none());
}

#[test]
fn tokens_request_modified_takes_litellms_buffers() {
    let request: ModifiedRequest = read(r#"{"model":"gpt-4o","base_model":"gpt-4o"}"#).unwrap();
    let settings = request.settings();
    assert_eq!(settings.buffer_fraction.to_bits(), 0.1_f64.to_bits());
    assert_eq!(settings.buffer_minimum_tokens.to_bits(), 10.0_f64.to_bits());
    assert!(request.user_max_tokens.is_none());
}

#[test]
fn tokens_request_model_reads_the_name() {
    let request: ModelRequest = read(r#"{"model":"gpt-4o"}"#).unwrap();
    assert_eq!(request.model, "gpt-4o");
    assert_eq!(ModelRequest::default().model, "");
}

#[test]
fn tokens_request_malformed_json_names_the_request() {
    let error = read::<TokenCountRequest>("{").unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("the token count request could not be read as JSON")
    );
    let wrong: Result<ResizeRequest, _> = read(r#"{"width":"wide"}"#);
    let error = wrong.unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("the image resize request could not be read as JSON")
    );
}
