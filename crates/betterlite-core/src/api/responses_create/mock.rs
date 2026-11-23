use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;

pub const DEFAULT_MOCK_TEXT: &str = "In a peaceful grove beneath a silver moon, a unicorn named Lumina discovered a hidden pool that reflected the stars. As she dipped her horn into the water, the pool began to shimmer, revealing a pathway to a magical realm of endless night skies. Filled with wonder, Lumina whispered a wish for all who dream to find their own hidden magic, and as she glanced back, her hoofprints sparkled like stardust.";

// mock_responses_api_response's literal body, with the one caller-chosen field left as a placeholder
const MOCK_RESPONSE: &str = r#"{
  "id": "resp_67ccd2bed1ec8190b14f964abc0542670bb6a6b452d3795b",
  "object": "response",
  "created_at": 1741476542,
  "status": "completed",
  "error": null,
  "incomplete_details": null,
  "instructions": null,
  "max_output_tokens": null,
  "model": "gpt-4.1-2025-04-14",
  "output": [
    {
      "type": "message",
      "id": "msg_67ccd2bf17f0819081ff3bb2cf6508e60bb6a6b452d3795b",
      "status": "completed",
      "role": "assistant",
      "content": [
        {"type": "output_text", "text": $TEXT, "annotations": []}
      ]
    }
  ],
  "parallel_tool_calls": true,
  "previous_response_id": null,
  "reasoning": {"effort": null, "summary": null},
  "store": true,
  "temperature": 1.0,
  "text": {"format": {"type": "text"}},
  "tool_choice": "auto",
  "tools": [],
  "top_p": 1.0,
  "truncation": "disabled",
  "usage": {
    "input_tokens": 36,
    "input_tokens_details": {"cached_tokens": 0},
    "output_tokens": 87,
    "output_tokens_details": {},
    "total_tokens": 123
  },
  "user": null,
  "metadata": {}
}"#;

pub fn mock_responses_api_response(
    mock_response: Option<&str>,
) -> Result<ResponsesApiResponse, serde_json::Error> {
    let text = serde_json::to_string(mock_response.unwrap_or(DEFAULT_MOCK_TEXT))?;
    serde_json::from_str(&MOCK_RESPONSE.replace("$TEXT", &text))
}

// _has_file_search_tool: any tool entry that is an object with type file_search
#[must_use]
pub fn has_file_search_tool(tools: Option<&[JsonValue]>) -> bool {
    tools.is_some_and(|present| {
        present.iter().any(|tool| {
            matches!(tool, JsonValue::Object(fields) if matches!(fields.get("type"), Some(JsonValue::Text(kind)) if kind == "file_search"))
        })
    })
}
