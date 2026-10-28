#![cfg(test)]

use super::{
    Message, PromptError, alpaca, claude, claude_2_1, default_pt, falcon::falcon_instruct,
    falcon_chat, granite, llama2, mistral, mpt, phind, titan, wizardcoder,
};
use rstest::rstest;
type Render = fn(&[Message]) -> Result<String, PromptError>;

#[rstest]
#[case::default(default_pt, r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#, Some("s u a"))]
#[case::alpaca(alpaca, r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#, Some("<s>### Instruction:\ns\n\n### Instruction:\nu\n\n### Response:\na\n\n</s>"))]
#[case::llama2(llama2, r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#, Some("<s>[INST] <<SYS>>\ns\n<</SYS>>\n [/INST]\n[INST] u [/INST]\na\n</s>"))]
#[case::mistral(mistral, r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#, Some("<s>[INST] \ns [/INST]\n[INST] u [/INST]\n a</s> "))]
#[case::falcon_instruct(falcon_instruct, r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#, Some("suser:u\n\nassistant:a\n\n"))]
#[case::falcon_chat(falcon_chat, r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#, Some("System: sUser: uFalcon: a"))]
#[case::mpt(mpt, r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#, Some("<|im_start|>systems<|im_end|>\n<|im_start|>useru<|im_end|>\n<|im_start|>assistanta<|im_end|>\n"))]
#[case::wizardcoder(wizardcoder, r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#, Some("s\n\n### Instruction:\nu\n\n### Response:\na\n\n"))]
#[case::phind(phind, r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#, Some("### System Prompt\ns\n\n### User Message\nu\n\n### Assistant\na\n\n"))]
#[case::claude(claude, r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#, Some("\n\nHuman: <admin>s</admin>\n\nHuman: u\n\nAssistant: a"))]
#[case::claude_2_1(claude_2_1, r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#, Some("s\n\nHuman: u\n\nAssistant: a"))]
#[case::granite(granite, r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#, Some("<|system|>\ns\n<|user|>\nu\n<|assistant|>\na"))]
#[case::titan(titan, r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#, Some("\n\nUser: <admin>s</admin>\n\nUser: u\n\nBot: a"))]
#[case::alpaca_reopen(alpaca, r#"[{"role":"assistant","content":"a"},{"role":"system","content":"s"},{"role":"human","content":"h"},{"role":"other","content":"x"},{"role":"user","content":[{"type":"text","text":"t"},{"type":"other","text":"z"},{"type":"image_url","image_url":"raw"}]}]"#, Some("<s>### Response:\na\n\n</s><s>### Instruction:\ns\n\nhx### Instruction:\ntz\n\n"))]
#[case::alpaca_empty(alpaca, "[]", Some("<s>"))]
#[case::llama2_reopen(llama2, r#"[{"role":"assistant","content":"a"},{"role":"system","content":"s"},{"role":"human","content":"h"},{"role":"other","content":"x"},{"role":"user","content":[{"type":"text","text":"t"},{"type":"other","text":"z"},{"type":"image_url","image_url":"raw"}]}]"#, Some("<s>a\n</s><s>[INST] <<SYS>>\ns\n<</SYS>>\n [/INST]\nhx[INST] tz [/INST]\n"))]
#[case::llama2_empty(llama2, "[]", Some("<s>"))]
#[case::mistral_reopen(mistral, r#"[{"role":"assistant","content":"a"},{"role":"system","content":"s"},{"role":"human","content":"h"},{"role":"other","content":"x"},{"role":"user","content":[{"type":"text","text":"t"},{"type":"other","text":"z"},{"type":"image_url","image_url":"raw"}]}]"#, Some("<s> a</s> [INST] \ns [/INST]\nhx[INST] tz [/INST]\n"))]
#[case::mistral_empty(mistral, "[]", Some("<s>"))]
#[case::granite_reopen(granite, r#"[{"role":"assistant","content":"a"},{"role":"system","content":"s"},{"role":"human","content":"h"},{"role":"other","content":"x"},{"role":"user","content":[{"type":"text","text":"t"},{"type":"other","text":"z"},{"type":"image_url","image_url":"raw"}]}]"#, Some("a\n<|system|>\ns\nhx<|user|>\ntz\n<|assistant|>"))]
#[case::granite_empty(granite, "[]", Some(""))]
#[case::falcon_chat_omission(falcon_chat, r#"[{"role":"tool","content":"x"}]"#, Some(""))]
#[case::mpt_omission(mpt, r#"[{"role":"tool","content":"x"}]"#, Some(""))]
#[case::wizardcoder_omission(wizardcoder, r#"[{"role":"tool","content":"x"}]"#, Some(""))]
#[case::phind_omission(phind, r#"[{"role":"tool","content":"x"}]"#, Some(""))]
#[case::claude_first_assistant(
    claude,
    r#"[{"role":"assistant","content":"a"}]"#,
    Some("\n\nHuman: \n\nAssistant: a")
)]
#[case::claude_system_assistant(
    claude,
    r#"[{"role":"system","content":"s"},{"role":"assistant","content":"a"}]"#,
    Some("\n\nHuman: <admin>s</admin>\n\nAssistant: a")
)]
#[case::claude_final_user(
    claude,
    r#"[{"role":"user","content":"u"}]"#,
    Some("\n\nHuman: u\n\nAssistant: ")
)]
#[case::claude_unknown(
    claude,
    r#"[{"role":"other","content":"x"}]"#,
    Some("\n\nAssistant: x\n\nAssistant: ")
)]
#[case::claude_empty(claude, "[]", None)]
#[case::claude21_first_assistant(
    claude_2_1,
    r#"[{"role":"assistant","content":"a"}]"#,
    Some("\n\nAssistant: a")
)]
#[case::claude21_system_assistant(
    claude_2_1,
    r#"[{"role":"system","content":"s"},{"role":"assistant","content":"a"}]"#,
    Some("s\n\nHuman: \n\nAssistant: a")
)]
#[case::claude21_final_user(
    claude_2_1,
    r#"[{"role":"user","content":"u"}]"#,
    Some("\n\nHuman: u\n\nAssistant: ")
)]
#[case::claude21_unknown(
    claude_2_1,
    r#"[{"role":"other","content":"x"}]"#,
    Some("\n\nAssistant: ")
)]
#[case::claude21_empty(claude_2_1, "[]", None)]
#[case::titan_first_assistant(
    titan,
    r#"[{"role":"assistant","content":"a"}]"#,
    Some("\n\nUser: \n\nBot: a")
)]
#[case::titan_system_assistant(
    titan,
    r#"[{"role":"system","content":"s"},{"role":"assistant","content":"a"}]"#,
    Some("\n\nUser: <admin>s</admin>\n\nBot: a")
)]
#[case::titan_final_user(
    titan,
    r#"[{"role":"user","content":"u"}]"#,
    Some("\n\nUser: u\n\nBot: ")
)]
#[case::titan_unknown(
    titan,
    r#"[{"role":"other","content":"x"}]"#,
    Some("\n\nBot: x\n\nBot: ")
)]
#[case::titan_empty(titan, "[]", None)]
#[case::default_empty(default_pt, "[]", Some(""))]
#[case::default_parts(default_pt, r#"[{"role":"user","content":[]}]"#, None)]
#[case::falcon_newlines(
    falcon_instruct,
    r#"[{"role":"user","content":"a\r\nb\n\nc\n\n\nd"}]"#,
    Some("user:a\nb\nc\n\nd\n\n")
)]
#[case::alpaca_null(alpaca, r#"[{"role":"assistant","content":null}]"#, Some("<s></s>"))]
#[case::llama2_null(llama2, r#"[{"role":"assistant","content":null}]"#, Some("<s></s>"))]
#[case::mistral_null(mistral, r#"[{"role":"assistant","content":null}]"#, Some("<s>"))]
#[case::granite_null(granite, r#"[{"role":"assistant","content":null}]"#, Some(""))]
#[case::claude_null(
    claude,
    r#"[{"role":"assistant","content":null}]"#,
    Some("\n\nHuman: \n\nAssistant: None")
)]
#[case::claude21_null(
    claude_2_1,
    r#"[{"role":"assistant","content":null}]"#,
    Some("\n\nAssistant: None")
)]
#[case::titan_null(
    titan,
    r#"[{"role":"assistant","content":null}]"#,
    Some("\n\nUser: \n\nBot: None")
)]
#[case::claude_parts(
    claude,
    r#"[{"role":"user","content":[{"type":"text","text":"don't\n"}]}]"#,
    Some("\n\nHuman: [{'type': 'text', 'text': \"don't\\n\"}]\n\nAssistant: ")
)]
#[case::claude21_parts(
    claude_2_1,
    r#"[{"role":"user","content":[{"type":"text","text":"don't\n"}]}]"#,
    Some("\n\nHuman: [{'type': 'text', 'text': \"don't\\n\"}]\n\nAssistant: ")
)]
#[case::titan_parts(
    titan,
    r#"[{"role":"user","content":[{"type":"text","text":"don't\n"}]}]"#,
    Some("\n\nUser: [{'type': 'text', 'text': \"don't\\n\"}]\n\nBot: ")
)]
#[case::alpaca_human_reopens(
    alpaca,
    r#"[{"role":"assistant","content":"a"},{"role":"human","content":"h"}]"#,
    Some("<s>### Response:\na\n\n</s><s>h")
)]
#[case::alpaca_user_no_bos(
    alpaca,
    r#"[{"role":"assistant","content":"a"},{"role":"user","content":"u"}]"#,
    Some("<s>### Response:\na\n\n</s>### Instruction:\nu\n\n")
)]
#[case::alpaca_systems(
    alpaca,
    r#"[{"role":"system","content":"s"},{"role":"system","content":"t"}]"#,
    Some("<s>### Instruction:\ns\n\n### Instruction:\nt\n\n")
)]
fn builtin_prompts_behavior_tests_behavior(
    #[case] render: Render,
    #[case] wire: &str,
    #[case] expected: Option<&str>,
) {
    let messages: Vec<Message> = serde_json::from_str(wire).unwrap();
    let actual = render(&messages);
    match expected {
        Some(text) => assert_eq!(actual.unwrap(), text),
        None => assert!(matches!(
            actual,
            Err(PromptError::Text { .. } | PromptError::Empty)
        )),
    }
}
