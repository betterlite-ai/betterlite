from typing import Literal, TypeAlias, TypedDict

from litellm.llms.base_llm.guardrail_translation.utils import (
    blocked_chat_stream_usage,
    blocked_response_usage,
    blocked_responses_api_usage,
    blocked_responses_stream_usage,
)
from litellm.types.llms.openai import ResponseAPIUsage

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class JsonEntry(TypedDict):
    kind: Literal["json"]
    value: Json


class BytesEntry(TypedDict):
    kind: Literal["bytes"]
    text: str


class AbsentOriginal(TypedDict):
    kind: Literal["absent"]


class ResponseOriginal(TypedDict):
    kind: Literal["response"]
    response: dict[str, Json]


class StreamOriginal(TypedDict):
    kind: Literal["stream"]
    items: list[JsonEntry | BytesEntry]


Original: TypeAlias = AbsentOriginal | ResponseOriginal | StreamOriginal


class BlockedUsageInput(TypedDict):
    function: Literal["blocked_usage"]
    shape: Literal["response", "responses_api", "chat_stream", "responses_stream"]
    original: Original


def stream_item(entry: JsonEntry | BytesEntry) -> Json | bytes:
    match entry["kind"]:
        case "json":
            return entry["value"]
        case "bytes":
            return entry["text"].encode()


def original_response(
    original: Original,
) -> dict[str, Json] | list[Json | bytes] | None:
    match original["kind"]:
        case "absent":
            return None
        case "response":
            return original["response"]
        case "stream":
            return [stream_item(entry) for entry in original["items"]]


def responses_usage(usage: ResponseAPIUsage) -> Json:
    return {
        "input_tokens": usage.input_tokens,
        "output_tokens": usage.output_tokens,
        "total_tokens": usage.total_tokens,
    }


def run(input_value: BlockedUsageInput) -> Json:
    original = original_response(input_value["original"])
    match input_value["shape"]:
        case "response":
            usage = blocked_response_usage(original)
            return {
                "input_tokens": usage.get("input_tokens"),
                "output_tokens": usage.get("output_tokens"),
            }
        case "chat_stream":
            prompt_tokens, completion_tokens = blocked_chat_stream_usage(original)
            return {"prompt_tokens": prompt_tokens, "completion_tokens": completion_tokens}
        case "responses_api":
            return responses_usage(blocked_responses_api_usage(original))
        case "responses_stream":
            return responses_usage(blocked_responses_stream_usage(original))
