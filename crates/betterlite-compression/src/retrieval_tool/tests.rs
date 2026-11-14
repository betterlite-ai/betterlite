#![cfg(test)]

mod retrieval_tool_schema {
    use super::super::*;
    use crate::message_normalization::CallType;
    use serde_json::json;

    #[test]
    fn builds_an_openai_function_tool_with_the_key_enum() {
        let keys = vec!["main.py".to_owned(), "message_3".to_owned()];
        assert_eq!(
            build_retrieval_tool(&keys),
            json!({
                "type": "function",
                "function": {
                    "name": "litellm_content_retrieve",
                    "description": "Retrieve the full content of a file or message that was compressed to save tokens. Use this when you need the complete content to answer accurately. Available keys: main.py, message_3",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "key": {
                                "type": "string",
                                "description": "The identifier of the content to retrieve",
                                "enum": ["main.py", "message_3"],
                            }
                        },
                        "required": ["key"],
                    },
                },
            })
        );
    }

    #[test]
    fn keeps_the_openai_schema_for_chat_completions() {
        let tools =
            build_retrieval_tools(&["main.py".to_owned()], CallType::Completion).expect("tools");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], RETRIEVAL_TOOL_NAME);
    }

    #[test]
    fn maps_to_the_anthropic_custom_tool_schema() {
        let keys = vec!["main.py".to_owned()];
        let tools = build_retrieval_tools(&keys, CallType::AnthropicMessages).expect("tools");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "custom");
        assert_eq!(tools[0]["name"], RETRIEVAL_TOOL_NAME);
        assert_eq!(tools[0]["input_schema"]["type"], "object");
        assert_eq!(
            tools[0]["input_schema"]["properties"]["key"]["enum"],
            json!(keys)
        );
        assert_eq!(tools[0]["input_schema"]["required"], json!(["key"]));
        assert_eq!(
            tools[0]["description"],
            "Retrieve the full content of a file or message that was compressed to save tokens. Use this when you need the complete content to answer accurately. Available keys: main.py"
        );
    }

    #[test]
    fn builds_no_tools_without_keys() {
        assert!(
            build_retrieval_tools(&[], CallType::Completion)
                .expect("tools")
                .is_empty()
        );
        assert!(
            build_retrieval_tools(&[], CallType::AnthropicMessages)
                .expect("tools")
                .is_empty()
        );
    }
}
