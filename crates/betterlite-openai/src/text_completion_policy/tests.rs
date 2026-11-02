#![cfg(test)]
use super::prompt;
use super::reply;
use super::{
    SUPPORTED_PARAMS, TextCompletionProviderTag, TextCompletionSettings,
    select_completion_provider, transform_completion_request,
};
use crate::base_helpers::provider_config;
use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;
use serde_json::{Map, json};

#[rstest]
#[case::text_config_values(0)]
#[case::text_parameter_allowlist(1)]
#[case::single_prompt_tokens(2)]
#[case::multiple_prompt_strings(3)]
#[case::text_reply_as_chat(4)]
#[case::text_config_selection(5)]
fn text_completion_policy_cases(#[case] case: u8) {
    match case {
        0 => {
            let settings: TextCompletionSettings = serde_json::from_value(json!({
                "max_tokens": 0_i32,
                "echo": false,
                "stop": [],
                "best_of": 3_i32,
                "temperature": 0.0_f64,
            }))
            .expect("settings parse");
            let config = provider_config(&settings).expect("config");
            assert_eq!(config.get("max_tokens"), Some(&json!(0_i32)));
            assert_eq!(config.get("echo"), Some(&json!(false)));
            assert_eq!(config.get("stop"), Some(&json!([])));
            assert_eq!(config.get("best_of"), Some(&json!(3_i32)));
            assert_eq!(config.get("temperature"), Some(&json!(0.0_f64)));
            assert!(!config.contains_key("logprobs"));
            assert!(!config.contains_key("suffix"));
            assert!(!config.contains_key("n"));

            let first: TextCompletionSettings =
                serde_json::from_value(json!({"echo": true})).expect("settings parse");
            let second: TextCompletionSettings =
                serde_json::from_value(json!({"echo": false})).expect("settings parse");
            assert_ne!(
                provider_config(&first).expect("config"),
                provider_config(&second).expect("config")
            );
        }
        1 => {
            assert_eq!(
                SUPPORTED_PARAMS,
                [
                    "functions",
                    "function_call",
                    "temperature",
                    "top_p",
                    "n",
                    "stream",
                    "stream_options",
                    "stop",
                    "max_tokens",
                    "presence_penalty",
                    "frequency_penalty",
                    "logit_bias",
                    "user",
                    "response_format",
                    "seed",
                    "tools",
                    "tool_choice",
                    "max_retries",
                    "logprobs",
                    "top_logprobs",
                    "extra_headers",
                ]
            );
            assert!(!SUPPORTED_PARAMS.contains(&"best_of"));
            assert!(!SUPPORTED_PARAMS.contains(&"echo"));
            assert!(!SUPPORTED_PARAMS.contains(&"suffix"));
        }
        2 => {
            assert_eq!(
                prompt::transform_prompt(&[
                    json!({"role": "user", "content": [1_i32, 2_i32, 3_i32]})
                ])
                .expect("tokens"),
                json!([1_i32, 2_i32, 3_i32])
            );
            assert_eq!(
                prompt::transform_prompt(&[
                    json!({"role": "user", "content": [[1_i32, 2_i32], [3_i32]]})
                ])
                .expect("list of tokens"),
                json!([[1_i32, 2_i32], [3_i32]])
            );
            assert_eq!(
                prompt::transform_prompt(&[json!({"role": "user", "content": [true, false]})])
                    .expect("bool counts as int"),
                json!([true, false])
            );
            assert!(prompt::is_tokens_or_list_of_tokens(&json!([])));
            assert!(prompt::is_tokens_or_list_of_tokens(&json!([[]])));
            assert_eq!(
                prompt::transform_prompt(&[json!({"role": "user", "content": []})])
                    .expect("empty content falls through"),
                json!("")
            );
            assert!(matches!(
                prompt::transform_prompt(&[json!({"role": "user", "content": [1_i32, "a"]})]),
                Err(prompt::PromptError::MalformedContent(_))
            ));
        }
        3 => {
            assert_eq!(
                prompt::transform_prompt(&[]).expect("zero messages"),
                json!([])
            );
            assert_eq!(
                prompt::transform_prompt(&[json!({"role": "user", "content": "hi"})])
                    .expect("one ordinary message"),
                json!("hi")
            );
            assert_eq!(
                prompt::transform_prompt(&[
                    json!({"role": "user", "content": "first"}),
                    json!({"role": "user", "content": "second"}),
                ])
                .expect("preserves order"),
                json!(["first", "second"])
            );
            assert!(
                prompt::transform_prompt(&[
                    json!({"role": "user", "content": [1_i32, 2_i32]}),
                    json!({"role": "user", "content": [3_i32, 4_i32]}),
                ])
                .is_err(),
                "token arrays never flatten across messages"
            );

            let optional_params =
                Map::from_iter([(String::from("model"), json!("override-model"))]);
            let request = transform_completion_request(
                "base-model",
                &[json!({"role": "user", "content": "hi"})],
                optional_params,
            )
            .expect("request");
            assert_eq!(request["model"], json!("override-model"));
            assert_eq!(request["prompt"], json!("hi"));
        }
        4 => {
            assert!(matches!(
                reply::to_chat_model_response(None, Some(Map::new())),
                Err(reply::ReplyError::MissingResponseFormat)
            ));
            assert!(matches!(
                reply::to_chat_model_response(Some(json!({})), None),
                Err(reply::ReplyError::MissingResponseFormat)
            ));

            let target = Map::from_iter([(String::from("object"), json!("text_completion"))]);
            let response = json!({"choices": [], "id": "cmpl-1"});
            let result = reply::to_chat_model_response(Some(response.clone()), Some(target))
                .expect("empty choices accepted");
            assert_eq!(result["choices"], json!([]));
            assert_eq!(result["id"], json!("cmpl-1"));
            assert_eq!(result["object"], json!("text_completion"));
            assert!(!result.contains_key("usage"));
            assert_eq!(result["_hidden_params"]["original_response"], response);

            let response_two = json!({
                "choices": [
                    {"text": "a", "finish_reason": "stop", "index": 9_i32},
                    {"text": "b", "finish_reason": "length"},
                ],
                "usage": {"total_tokens": 3_i32},
            });
            let result_two = reply::to_chat_model_response(Some(response_two), Some(Map::new()))
                .expect("sequential index");
            assert_eq!(result_two["choices"][0]["index"], json!(0_i32));
            assert_eq!(result_two["choices"][1]["index"], json!(1_i32));
            assert_eq!(
                result_two["choices"][0]["message"],
                json!({"role": "assistant", "content": "a"})
            );
            assert_eq!(result_two["choices"][1]["logprobs"], json!(null));
            assert_eq!(result_two["usage"], json!({"total_tokens": 3_i32}));
            assert!(!result_two.contains_key("id"));
            assert!(!result_two.contains_key("model"));

            let malformed = json!({"choices": [{"finish_reason": "stop"}]});
            assert!(matches!(
                reply::to_chat_model_response(Some(malformed), Some(Map::new())),
                Err(reply::ReplyError::MalformedChoices(_))
            ));
        }
        5 => {
            assert_eq!(
                select_completion_provider(ProviderId::FireworksAi),
                TextCompletionProviderTag::FireworksAi
            );
            assert_eq!(
                select_completion_provider(ProviderId::TogetherAi),
                TextCompletionProviderTag::TogetherAi
            );
            assert_eq!(
                select_completion_provider(ProviderId::TextCompletionInception),
                TextCompletionProviderTag::TextCompletionInception
            );
            assert_eq!(
                select_completion_provider(ProviderId::Openai),
                TextCompletionProviderTag::OpenAi
            );
            assert_eq!(
                select_completion_provider(ProviderId::Anthropic),
                TextCompletionProviderTag::OpenAi
            );
        }
        _ => panic!("unknown case {case}"),
    }
}
