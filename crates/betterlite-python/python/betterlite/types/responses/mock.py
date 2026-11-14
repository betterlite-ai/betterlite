from typing import Final

from pydantic import JsonValue

from betterlite.types.responses.api_response import ResponsesAPIResponse

DEFAULT_RESPONSE_TEXT: Final = (
    "In a peaceful grove beneath a silver moon, a unicorn named Lumina discovered a hidden pool "
    "that reflected the stars. As she dipped her horn into the water, the pool began to shimmer, "
    "revealing a pathway to a magical realm of endless night skies. Filled with wonder, Lumina "
    "whispered a wish for all who dream to find their own hidden magic, and as she glanced back, "
    "her hoofprints sparkled like stardust."
)


def mock_responses_api_response(
    mock_response: str = DEFAULT_RESPONSE_TEXT,
) -> ResponsesAPIResponse:
    body: dict[str, JsonValue] = {
        "id": "resp_67ccd2bed1ec8190b14f964abc0542670bb6a6b452d3795b",
        "object": "response",
        "created_at": 1741476542,
        "status": "completed",
        "error": None,
        "incomplete_details": None,
        "instructions": None,
        "max_output_tokens": None,
        "model": "gpt-4.1-2025-04-14",
        "output": [
            {
                "type": "message",
                "id": "msg_67ccd2bf17f0819081ff3bb2cf6508e60bb6a6b452d3795b",
                "status": "completed",
                "role": "assistant",
                "content": [
                    {
                        "type": "output_text",
                        "text": mock_response,
                        "annotations": [],
                        "logprobs": None,
                    },
                ],
                "phase": None,
            },
        ],
        "parallel_tool_calls": True,
        "previous_response_id": None,
        "reasoning": {"effort": None, "summary": None},
        "store": True,
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
            "total_tokens": 123,
        },
        "user": None,
        "metadata": {},
    }
    return ResponsesAPIResponse.model_validate(body)
