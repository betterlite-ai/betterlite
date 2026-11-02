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
use uuid::Uuid;

fn fixed_identifier() -> Uuid {
    Uuid::from_u128(0x0123_4567_89ab_4def_8123_4567_89ab_cdef)
}

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
                prompt::transform_prompt(&[json!({"role": "user", "content": [-1_i32, -2_i32]})])
                    .expect("negative ints count as int-like"),
                json!([-1_i32, -2_i32])
            );
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
            let zero_usage = json!({
                "prompt_tokens": 0_i32,
                "completion_tokens": 0_i32,
                "total_tokens": 0_i32,
                "completion_tokens_details": null,
                "prompt_tokens_details": null,
            });
            let generated_id = format!("chatcmpl-{}", fixed_identifier());

            assert!(matches!(
                reply::to_chat_model_response(None, Some(Map::new()), &fixed_identifier),
                Err(reply::ReplyError::MissingResponseFormat)
            ));
            assert!(matches!(
                reply::to_chat_model_response(Some(json!({})), None, &fixed_identifier),
                Err(reply::ReplyError::MissingResponseFormat)
            ));

            let target = Map::from_iter([(String::from("object"), json!("text_completion"))]);
            let response = json!({"choices": [], "id": "cmpl-1"});
            let result = reply::to_chat_model_response(
                Some(response.clone()),
                Some(target),
                &fixed_identifier,
            )
            .expect("empty choices accepted");
            assert_eq!(result["choices"], json!([]));
            assert_eq!(result["id"], json!("cmpl-1"));
            assert_eq!(result["object"], json!("text_completion"));
            assert_eq!(result["usage"], zero_usage);
            assert_eq!(result["model"], json!(null));
            assert_eq!(
                result["_hidden_params"]["original_response"],
                json!({"choices": [], "id": "cmpl-1", "usage": zero_usage})
            );

            let response_two = json!({
                "choices": [
                    {"text": "a", "finish_reason": "stop", "index": 9_i32},
                    {"text": "b", "finish_reason": "length"},
                ],
                "usage": {"total_tokens": 3_i32},
            });
            let result_two = reply::to_chat_model_response(
                Some(response_two),
                Some(Map::new()),
                &fixed_identifier,
            )
            .expect("sequential index");
            assert_eq!(result_two["choices"][0]["index"], json!(0_i32));
            assert_eq!(result_two["choices"][1]["index"], json!(1_i32));
            assert_eq!(
                result_two["choices"][0]["message"],
                json!({
                    "role": "assistant",
                    "content": "a",
                    "tool_calls": null,
                    "function_call": null,
                })
            );
            assert!(
                !result_two["choices"][1]
                    .as_object()
                    .expect("choice object")
                    .contains_key("logprobs")
            );
            assert_eq!(
                result_two["usage"],
                json!({
                    "prompt_tokens": 0_i32,
                    "completion_tokens": 0_i32,
                    "total_tokens": 3_i32,
                    "completion_tokens_details": null,
                    "prompt_tokens_details": null,
                })
            );
            assert_eq!(result_two["id"], json!(generated_id));
            assert_eq!(result_two["model"], json!(null));

            let usage_missing = json!({"choices": [{"text": "a"}]});
            let result_usage_missing = reply::to_chat_model_response(
                Some(usage_missing),
                Some(Map::new()),
                &fixed_identifier,
            )
            .expect("missing usage defaults to zero");
            assert_eq!(result_usage_missing["usage"], zero_usage);

            let choices_missing = json!({"id": "cmpl-missing-choices"});
            let result_choices_missing = reply::to_chat_model_response(
                Some(choices_missing),
                Some(Map::new()),
                &fixed_identifier,
            )
            .expect("missing choices default to one empty choice");
            assert_eq!(
                result_choices_missing["choices"],
                json!([{
                    "finish_reason": "stop",
                    "index": 0_i32,
                    "message": {
                        "content": null,
                        "role": "assistant",
                        "tool_calls": null,
                        "function_call": null,
                    },
                }])
            );
            assert_eq!(
                result_choices_missing["_hidden_params"]["original_response"]["choices"],
                json!([{"finish_reason": null, "index": 0_i32, "text": null, "logprobs": null}])
            );

            let choices_null = json!({"id": "cmpl-null-choices", "choices": null});
            let result_choices_null = reply::to_chat_model_response(
                Some(choices_null),
                Some(Map::new()),
                &fixed_identifier,
            )
            .expect("null choices default to one empty choice");
            assert_eq!(
                result_choices_null["choices"],
                result_choices_missing["choices"]
            );

            let missing_fields = json!({"choices": [{}]});
            let result_three = reply::to_chat_model_response(
                Some(missing_fields),
                Some(Map::new()),
                &fixed_identifier,
            )
            .expect("missing choice fields default");
            assert_eq!(result_three["choices"][0]["finish_reason"], json!("stop"));
            assert_eq!(
                result_three["choices"][0]["message"]["content"],
                json!(null)
            );
            assert!(
                !result_three["choices"][0]
                    .as_object()
                    .expect("choice object")
                    .contains_key("logprobs")
            );

            let falsy_finish_reasons = json!({
                "choices": [
                    {"text": "a", "finish_reason": ""},
                    {"text": "b", "finish_reason": 0_i32},
                    {"text": "c", "finish_reason": false},
                ],
            });
            let result_falsy = reply::to_chat_model_response(
                Some(falsy_finish_reasons),
                Some(Map::new()),
                &fixed_identifier,
            )
            .expect("falsy finish_reason collapses to stop");
            assert_eq!(result_falsy["choices"][0]["finish_reason"], json!("stop"));
            assert_eq!(result_falsy["choices"][1]["finish_reason"], json!("stop"));
            assert_eq!(result_falsy["choices"][2]["finish_reason"], json!("stop"));
            assert_eq!(
                result_falsy["_hidden_params"]["original_response"]["choices"][0]["finish_reason"],
                json!(null)
            );

            let mapped_finish_reasons = json!({
                "choices": [
                    {"text": "a", "finish_reason": "tool_use"},
                    {"text": "b", "finish_reason": "stop_sequence"},
                    {"text": "c", "finish_reason": "my_custom_reason"},
                    {"text": "d", "finish_reason": "refusal"},
                    {"text": "e", "finish_reason": "function_call"},
                ],
            });
            let result_mapped = reply::to_chat_model_response(
                Some(mapped_finish_reasons),
                Some(Map::new()),
                &fixed_identifier,
            )
            .expect("raw provider reasons map through the table");
            assert_eq!(
                result_mapped["choices"][0]["finish_reason"],
                json!("tool_calls")
            );
            assert_eq!(result_mapped["choices"][1]["finish_reason"], json!("stop"));
            assert_eq!(result_mapped["choices"][2]["finish_reason"], json!("stop"));
            assert_eq!(
                result_mapped["choices"][3]["finish_reason"],
                json!("content_filter")
            );
            assert_eq!(
                result_mapped["choices"][4]["finish_reason"],
                json!("function_call")
            );
            assert_eq!(
                result_mapped["_hidden_params"]["original_response"]["choices"][0]["finish_reason"],
                json!("tool_calls")
            );
            assert_eq!(
                result_mapped["_hidden_params"]["original_response"]["choices"][1]["finish_reason"],
                json!("stop")
            );

            let explicit_null_logprobs = json!({"choices": [{"text": "d", "logprobs": null}]});
            let result_explicit_null = reply::to_chat_model_response(
                Some(explicit_null_logprobs),
                Some(Map::new()),
                &fixed_identifier,
            )
            .expect("explicit null logprobs omits the field");
            assert!(
                !result_explicit_null["choices"][0]
                    .as_object()
                    .expect("choice object")
                    .contains_key("logprobs")
            );

            let with_logprobs = json!({
                "choices": [{"text": "c", "finish_reason": "length", "logprobs": {"tokens": ["c"]}}],
            });
            let result_four = reply::to_chat_model_response(
                Some(with_logprobs),
                Some(Map::new()),
                &fixed_identifier,
            )
            .expect("explicit logprobs pass through");
            assert_eq!(
                result_four["choices"][0]["logprobs"],
                json!({"tokens": ["c"]})
            );
            assert_eq!(result_four["choices"][0]["finish_reason"], json!("length"));

            let malformed = json!({"choices": [{"text": 5_i32, "finish_reason": "stop"}]});
            assert!(matches!(
                reply::to_chat_model_response(Some(malformed), Some(Map::new()), &fixed_identifier),
                Err(reply::ReplyError::MalformedChoices(_))
            ));
            let malformed_shape = json!({"choices": ["not-an-object"]});
            assert!(matches!(
                reply::to_chat_model_response(
                    Some(malformed_shape),
                    Some(Map::new()),
                    &fixed_identifier
                ),
                Err(reply::ReplyError::MalformedChoices(_))
            ));

            let target_with_hidden_params = Map::from_iter([(
                String::from("_hidden_params"),
                json!({"model_id": "existing-id"}),
            )]);
            let response_three = json!({"choices": []});
            let result_five = reply::to_chat_model_response(
                Some(response_three.clone()),
                Some(target_with_hidden_params),
                &fixed_identifier,
            )
            .expect("preserves other hidden params");
            assert_eq!(
                result_five["_hidden_params"]["model_id"],
                json!("existing-id")
            );
            assert_eq!(
                result_five["_hidden_params"]["original_response"],
                json!({"choices": [], "id": generated_id, "usage": zero_usage})
            );
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
