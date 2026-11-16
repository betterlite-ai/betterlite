#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::ORACLE_DIR;

fn load_case(file: &str) -> HelperCase {
    let path = Path::new(ORACLE_DIR)
        .join("cases/helpers/fireworks-ai-completion-responses")
        .join(file);
    serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn run_case_file(file: &str) -> Value {
    use betterlite_oracle_helper_api::helper::Helper as _;

    super::Op::decode_and_run(&load_case(file).call.input).unwrap()
}

#[test]
fn the_supported_list_is_the_completion_param_list() {
    let answers = run_case_file("completion-params.yaml");
    assert_eq!(
        answers[0],
        json!([
            "max_tokens",
            "logprobs",
            "echo",
            "temperature",
            "top_p",
            "top_k",
            "frequency_penalty",
            "presence_penalty",
            "n",
            "stop",
            "response_format",
            "stream",
            "user",
        ])
    );
    assert_eq!(
        answers[5],
        json!({
            "provider": "fireworks_ai",
            "native_websocket": false,
            "native_file_search": false,
        })
    );
}

#[test]
fn map_openai_params_copies_only_supported_names_and_keeps_null_values() {
    let answers = run_case_file("completion-params.yaml");
    assert_eq!(
        answers[1],
        json!({
            "existing": 1,
            "max_tokens": 10,
            "logprobs": true,
            "echo": false,
            "temperature": 0.2,
            "top_p": 0.9,
            "top_k": 40,
            "frequency_penalty": 0.5,
            "presence_penalty": 0.1,
            "n": 2,
            "stop": ["\n"],
            "response_format": {"type": "json_object"},
            "stream": true,
            "user": "u1",
        })
    );
    assert_eq!(answers[2], json!({"max_tokens": null, "temperature": null}));
    assert_eq!(answers[3], json!({"temperature": 0.3}));
    assert_eq!(answers[4], json!({"logprobs": null}));
}

#[test]
fn the_strip_and_move_steps_clean_the_extra_body() {
    let answers = run_case_file("completion-extra-body.yaml");
    assert_eq!(
        answers[0],
        json!({"temperature": 0.5, "extra_body": {"custom": {"kept": true}}})
    );
    assert_eq!(answers[1], json!({"extra_body": {"custom": "kept"}}));
    assert_eq!(answers[2], json!({"temperature": 0.1}));
    assert_eq!(answers[3], json!({"temperature": 0.1}));
    assert_eq!(
        answers[4],
        json!({"extra_body": {
            "response_format": {"type": "json_object"},
            "thinking": {"type": "enabled"},
            "kept": 1,
            "reasoning_effort": "low",
        }})
    );
    assert_eq!(
        answers[5],
        json!({"extra_body": {
            "response_format": "kept",
            "reasoning_effort": "kept",
            "thinking": "kept",
        }})
    );
    assert_eq!(
        answers[6],
        json!({"extra_body": {"response_format": {"type": "json_object"}}})
    );
    assert_eq!(answers[7], json!({"extra_body": {"response_format": null}}));
}

#[test]
fn chat_template_kwargs_that_cannot_apply_are_dropped() {
    let answers = run_case_file("completion-extra-body.yaml");
    assert_eq!(answers[8], json!({"extra_body": {"custom": "kept"}}));
    assert_eq!(answers[9], json!({"extra_body": {"custom": "kept"}}));
    assert_eq!(
        answers[10],
        json!({"extra_body": {"chat_template_kwargs": null, "custom": "kept"}})
    );
    assert_eq!(
        answers[11],
        json!({"extra_body": {"custom": 1, "reasoning_effort": "high"}})
    );
    assert_eq!(
        answers[12],
        json!({"extra_body": {"thinking": {"type": "enabled"}, "custom": 1}})
    );
    assert_eq!(answers[13], json!({}));
}

#[test]
fn a_reasoning_model_turns_the_kwarg_effort_into_reasoning_effort() {
    let answers = run_case_file("completion-template-kwargs.yaml");
    assert_eq!(answers[0], json!({"extra_body": {"reasoning_effort": 100}}));
    assert_eq!(
        answers[1],
        json!({"extra_body": {"reasoning_effort": "none"}})
    );
    assert_eq!(
        answers[2],
        json!({"extra_body": {"reasoning_effort": "none"}})
    );
    assert_eq!(
        answers[3],
        json!({"extra_body": {"reasoning_effort": "none"}})
    );
    assert_eq!(answers[4], json!({"extra_body": {"reasoning_effort": 0}}));
    assert_eq!(answers[5], json!({"extra_body": {"reasoning_effort": 2}}));
    assert_eq!(answers[6], json!({}));
    assert_eq!(answers[7], json!({}));
    assert_eq!(
        answers[8],
        json!({"extra_body": {"reasoning_effort": "low"}})
    );
    assert_eq!(answers[9], json!({}));
    assert_eq!(
        answers[10],
        json!({"extra_body": {"custom": "kept", "reasoning_effort": "none"}})
    );
    assert_eq!(answers[17], json!({"extra_body": {"reasoning_effort": 3}}));
    assert_eq!(
        answers[18],
        json!({"extra_body": {"reasoning_effort": "low"}})
    );
    assert_eq!(
        answers[19],
        json!({"extra_body": {"reasoning_effort": "none"}})
    );
}

#[test]
fn chat_template_kwargs_shapes_that_do_not_apply_are_dropped() {
    let answers = run_case_file("completion-template-kwargs.yaml");
    assert_eq!(answers[11], json!({"extra_body": {"custom": "kept"}}));
    assert_eq!(
        answers[12],
        json!({"extra_body": {"chat_template_kwargs": null, "custom": "kept"}})
    );
}

#[test]
fn an_explicit_reasoning_param_blocks_the_kwarg_effort() {
    let answers = run_case_file("completion-template-kwargs.yaml");
    assert_eq!(
        answers[13],
        json!({"extra_body": {"reasoning_effort": "high"}})
    );
    assert_eq!(
        answers[14],
        json!({"extra_body": {"thinking": {"type": "enabled"}}})
    );
}

#[test]
fn a_model_without_reasoning_support_drops_the_kwarg_effort() {
    let answers = run_case_file("completion-template-kwargs.yaml");
    assert_eq!(answers[15], json!({"extra_body": {"custom": "kept"}}));
    assert_eq!(answers[16], json!({"extra_body": {"custom": "kept"}}));
}

#[test]
fn guided_params_become_one_response_format() {
    let answers = run_case_file("completion-guided.yaml");
    assert_eq!(
        answers[0],
        json!({"extra_body": {
            "custom": 1,
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "response",
                    "schema": {
                        "type": "object",
                        "properties": {"name": {"type": "string"}},
                    },
                },
            },
        }})
    );
    assert_eq!(
        answers[1],
        json!({"extra_body": {
            "custom": 1,
            "response_format": {"type": "grammar", "grammar": "root ::= \"a\""},
        }})
    );
    assert_eq!(
        answers[2],
        json!({"extra_body": {
            "custom": 1,
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "choice",
                    "schema": {"type": "string", "enum": ["a", "b"]},
                },
            },
        }})
    );
    assert_eq!(
        answers[3],
        json!({"extra_body": {
            "custom": 1,
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "response",
                    "schema": {"type": "object"},
                },
            },
        }})
    );
    assert_eq!(
        answers[4],
        json!({"extra_body": {
            "custom": 1,
            "response_format": {"type": "grammar", "grammar": "root ::= x"},
        }})
    );
    assert_eq!(answers[5], json!({"extra_body": {"custom": 1}}));
    assert_eq!(
        answers[6],
        json!({"extra_body": {
            "custom": 1,
            "response_format": {"type": "json_object"},
        }})
    );
    assert_eq!(
        answers[7],
        json!({"extra_body": {"response_format": "x", "custom": 1}})
    );
    assert_eq!(
        answers[8],
        json!({"extra_body": {
            "custom": 1,
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "choice",
                    "schema": {"type": "string", "enum": []},
                },
            },
        }})
    );
    assert_eq!(
        answers[9],
        json!({"extra_body": {
            "custom": 1,
            "response_format": {
                "type": "json_schema",
                "json_schema": {"name": "response", "schema": false},
            },
        }})
    );
}

#[test]
fn the_completion_request_resolves_the_model_and_projects_the_prompt() {
    let answers = run_case_file("completion-request.yaml");
    assert_eq!(
        answers[0],
        json!({
            "model": "accounts/fireworks/models/llama-v3p1-8b-instruct",
            "prompt": "hello",
            "temperature": 0.2,
            "max_tokens": 5,
            "stop": ["\n"],
            "extra_body": {
                "custom": 1,
                "response_format": {"type": "json_object"},
            },
        })
    );
    assert_eq!(
        answers[1],
        json!({"model": "accounts/fireworks/models/x", "prompt": ""})
    );
    assert_eq!(
        answers[2],
        json!({"model": "accounts/fireworks/models/y", "prompt": ""})
    );
    assert_eq!(
        answers[3],
        json!({"model": "accounts/fireworks/models/z", "prompt": [1, 2]})
    );
    assert_eq!(
        answers[4],
        json!({"model": "accounts/fireworks/routers/auto", "prompt": [[1, 2], [3]]})
    );
    assert_eq!(
        answers[5],
        json!({
            "model": "accounts/fireworks/routers/my-model-fast",
            "prompt": ["a", "b"],
        })
    );
    assert_eq!(
        answers[6],
        json!({"model": "accounts/fireworks/routers/firerouter/x", "prompt": "x"})
    );
    assert_eq!(
        answers[7],
        json!({"model": "FW-foundry#route", "prompt": "x"})
    );
    assert_eq!(
        answers[8],
        json!({"model": "accounts/fireworks/routers/auto", "prompt": [0]})
    );
    assert_eq!(
        answers[9],
        json!({"model": "accounts/fireworks/models/empty-model", "prompt": []})
    );
    assert_eq!(
        answers[10],
        json!({
            "model": "accounts/fireworks/models/stream-model",
            "prompt": "s",
            "stream": true,
            "seed": 7,
            "user": "u",
        })
    );
    assert_eq!(
        answers[11],
        json!({
            "model": "accounts/fireworks/models/m",
            "prompt": "x",
            "extra_body": {
                "thinking": {"type": "enabled"},
                "reasoning_effort": "low",
            },
        })
    );
}

#[test]
fn the_headers_start_with_the_json_content_type_then_the_callers_then_bearer() {
    let answers = run_case_file("responses-auth-and-url.yaml");
    assert_eq!(
        answers[0],
        json!({
            "provider": "fireworks_ai",
            "native_websocket": false,
            "native_file_search": false,
        })
    );
    assert_eq!(
        answers[1],
        json!({
            "Content-Type": "application/json",
            "X-Test": "1",
            "Authorization": "Bearer arg-key",
        })
    );
    assert_eq!(
        answers[2],
        json!({
            "Content-Type": "text/plain",
            "authorization": "Bearer caller",
            "X-Session-Affinity": "caller-pin",
            "Authorization": "Bearer arg-key",
        })
    );
}

#[test]
fn the_env_key_chain_skips_empty_values_and_the_argument() {
    let answers = run_case_file("responses-auth-and-url.yaml");
    assert_eq!(
        answers[3],
        json!({
            "Content-Type": "application/json",
            "Authorization": "Bearer env-secondary-key",
        })
    );
    assert_eq!(answers[4], answers[3]);
}

#[test]
fn a_session_id_pins_the_request_unless_it_was_generated() {
    let answers = run_case_file("responses-auth-and-url.yaml");
    assert_eq!(answers[5]["x-session-affinity"], "sess-1");
    assert!(answers[6].get("x-session-affinity").is_none());
    assert_eq!(answers[7]["x-session-affinity"], "sess-meta");
    assert!(answers[8].get("x-session-affinity").is_none());
}

#[test]
fn the_complete_url_takes_the_argument_then_the_environment() {
    let answers = run_case_file("responses-auth-and-url.yaml");
    assert_eq!(answers[9], "https://env.example/v1/responses");
    assert_eq!(answers[10], "https://env.example/v1/responses");
    assert_eq!(answers[11], "https://arg.example/v2/responses");
    assert_eq!(answers[12], "https://arg.example/v2/responses");
}

#[test]
fn instruction_items_fold_into_the_instructions() {
    let answers = run_case_file("responses-instruction-folding.yaml");
    assert_eq!(
        answers[0],
        json!({
            "model": "accounts/fireworks/models/bare-name",
            "input": "hi",
            "temperature": 0.5,
            "instructions": "sys",
        })
    );
    assert_eq!(
        answers[1],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [{"role": "user", "content": "u1"}],
            "instructions": "top\n\nsys-a",
        })
    );
    assert_eq!(
        answers[2],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [{"role": "user", "content": "u"}],
            "instructions": "d1\n\nd2\n\nd3",
        })
    );
    assert_eq!(answers[3]["instructions"], "i\n\nd");
    assert_eq!(
        answers[3]["input"],
        json!([
            {"role": "system", "content": [{"type": "input_image", "image_url": "x"}]},
            {"role": "user", "content": "u"},
        ])
    );
    assert_eq!(
        answers[4],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [{"role": "user", "content": "u"}],
            "instructions": "d1\n\nd2",
        })
    );
    assert_eq!(
        answers[5],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [
                {"role": "user", "content": "u"},
                {"role": "assistant", "content": "a"},
                {"role": "system", "content": "d", "type": "message"},
            ],
            "instructions": "i",
        })
    );
    assert_eq!(answers[6]["instructions"], "d");
}

#[test]
fn folded_texts_skip_empty_chunks_and_leave_no_instructions() {
    let answers = run_case_file("responses-instruction-folding.yaml");
    assert_eq!(answers[7]["instructions"], "i\n\ns1");
    assert!(answers[8].get("instructions").is_none());
    assert_eq!(answers[8]["input"], json!([]));
    assert!(answers[9].get("instructions").is_none());
    assert_eq!(answers[9]["input"], json!([]));
    assert_eq!(answers[10]["instructions"], 5);
    assert_eq!(
        answers[10]["input"],
        json!([{"role": "system", "content": "d", "type": "message"}])
    );
    assert_eq!(answers[11]["instructions"], "s");
    assert_eq!(answers[11]["input"], json!([]));
    assert_eq!(
        answers[12]["input"],
        json!([{"role": "user", "content": "u", "custom": 2}])
    );
    assert_eq!(
        answers[13],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": "hi",
        })
    );
    assert_eq!(answers[14]["instructions"], 5);
}

#[test]
fn a_non_text_part_keeps_its_item_and_a_developer_item_is_renamed() {
    let answers = run_case_file("responses-instruction-folding.yaml");
    assert_eq!(
        answers[15],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [{
                "role": "system",
                "content": [
                    {"type": "input_text", "text": "d2"},
                    {"type": "text", "text": "no"},
                ],
                "type": "message",
            }],
            "instructions": "d1",
        })
    );
    assert_eq!(answers[16]["instructions"], "s\n\nx\n\ny");
    assert_eq!(answers[16]["input"], json!([]));
    assert_eq!(answers[17]["instructions"], "s");
    assert_eq!(answers[17]["input"], json!([]));
}

#[test]
fn the_closing_assistant_stops_later_instruction_items_from_folding() {
    let answers = run_case_file("responses-closing-assistant.yaml");
    assert_eq!(
        answers[0],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [
                {"role": "user", "content": "u"},
                {"role": "assistant", "content": "a"},
                {"role": "system", "content": "tail", "type": "message"},
            ],
            "instructions": "i\n\ns\n\nd",
        })
    );
    assert_eq!(
        answers[1],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [
                {"role": "assistant", "content": "a"},
                {"role": "user", "content": "u"},
                {"role": "assistant", "content": "a2"},
                {"role": "system", "content": "late"},
            ],
            "instructions": "d",
        })
    );
    assert_eq!(
        answers[2],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [
                {"role": "user", "content": "u"},
                {"role": "assistant", "content": "a"},
                {"role": "system", "content": "s"},
                {"role": "system", "content": "d", "type": "message"},
            ],
            "instructions": "i",
        })
    );
    assert_eq!(
        answers[3],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [
                {"role": "assistant", "content": "a"},
                {"role": "system", "content": "d", "type": "message"},
            ],
            "instructions": "s",
        })
    );
    assert_eq!(
        answers[4],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [
                {"role": "assistant", "content": "a"},
                {"role": "user", "content": "u"},
            ],
            "instructions": "d",
        })
    );
    assert_eq!(
        answers[5],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [
                {"role": "assistant", "content": "a"},
                {"role": "user", "content": "u"},
            ],
            "instructions": "d",
        })
    );
    assert_eq!(
        answers[6],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [],
            "instructions": "i\n\ns\n\nd",
        })
    );
    assert_eq!(
        answers[7],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [
                {"role": "user", "content": "u"},
                {"role": "assistant", "content": "a"},
            ],
        })
    );
    assert_eq!(
        answers[8],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [{
                "role": "assistant",
                "content": [{"type": "input_text", "text": "a"}],
            }],
            "instructions": "s",
        })
    );
    assert_eq!(
        answers[9],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [],
            "instructions": "d\n\ns\n\ne",
        })
    );
    assert_eq!(
        answers[10],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [
                {"role": "user", "content": "u"},
                {"role": "assistant", "content": "a"},
                {"role": "system", "content": "s"},
            ],
        })
    );
    assert_eq!(
        answers[11],
        json!({
            "model": "accounts/fireworks/models/fold",
            "input": [
                {"role": "user", "content": "u"},
                {"role": "assistant", "content": "a"},
                {"role": "system", "content": "d1", "type": "message"},
                {"role": "system", "content": "d2", "type": "message"},
            ],
            "instructions": "i\n\ns",
        })
    );
}

#[test]
fn the_deleted_id_is_the_decoded_last_path_segment() {
    let answers = run_case_file("responses-delete-result.yaml");
    assert_eq!(
        answers[0],
        json!({"id": "resp_abc", "object": "response", "deleted": true})
    );
    let ids: Vec<Value> = answers
        .as_array()
        .unwrap()
        .iter()
        .map(|answer| answer["id"].clone())
        .collect();
    assert_eq!(
        ids,
        vec![
            json!("resp_abc"),
            json!("b"),
            json!("a/b"),
            json!("a b"),
            json!("\u{fffd}"),
            json!("a+b"),
            json!("%zz"),
            json!("A"),
            json!("%zz"),
            json!("%25"),
            json!("\u{20ac}"),
            json!(""),
            json!(""),
            json!(""),
            json!(""),
        ]
    );
    let results: Vec<(Value, Value)> = answers
        .as_array()
        .unwrap()
        .iter()
        .map(|answer| (answer["object"].clone(), answer["deleted"].clone()))
        .collect();
    assert_eq!(results, vec![(json!("response"), json!(true)); 15]);
}
