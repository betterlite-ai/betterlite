import logging
from typing import Final, Literal, TypeAlias

FinishReason: TypeAlias = Literal[
    "stop",
    "length",
    "tool_calls",
    "content_filter",
    "function_call",
]

FINISH_REASON_MAP: Final[dict[str, FinishReason]] = {
    "stop_sequence": "stop",
    "end_turn": "stop",
    "max_tokens": "length",
    "tool_use": "tool_calls",
    "refusal": "content_filter",
    "compaction": "length",
    "COMPLETE": "stop",
    "ERROR_TOXIC": "content_filter",
    "ERROR": "stop",
    "eos_token": "stop",
    "eos": "stop",
    "STOP": "stop",
    "MAX_TOKENS": "length",
    "SAFETY": "content_filter",
    "RECITATION": "content_filter",
    "FINISH_REASON_UNSPECIFIED": "stop",
    "MALFORMED_FUNCTION_CALL": "stop",
    "LANGUAGE": "content_filter",
    "OTHER": "content_filter",
    "BLOCKLIST": "content_filter",
    "PROHIBITED_CONTENT": "content_filter",
    "SPII": "content_filter",
    "IMAGE_SAFETY": "content_filter",
    "IMAGE_PROHIBITED_CONTENT": "content_filter",
    "TOO_MANY_TOOL_CALLS": "stop",
    "MALFORMED_RESPONSE": "stop",
    "NO_IMAGE": "content_filter",
    "IMAGE_RECITATION": "content_filter",
    "IMAGE_OTHER": "content_filter",
    "ESCALATION": "content_filter",
    "UNEXPECTED_TOOL_CALL": "stop",
    "MISSING_THOUGHT_SIGNATURE": "stop",
    "network_error": "stop",
    "sensitive": "content_filter",
    "guardrail_intervened": "content_filter",
    "stop": "stop",
    "length": "length",
    "tool_calls": "tool_calls",
    "function_call": "function_call",
    "content_filter": "content_filter",
    "content_filtered": "content_filter",
    "error": "stop",
}

LOGGER: Final = logging.getLogger("LiteLLM")


def map_finish_reason(finish_reason: str) -> FinishReason:
    mapped = FINISH_REASON_MAP.get(finish_reason)
    if mapped is None:
        LOGGER.warning("Unmapped finish_reason '%s', defaulting to 'stop'", finish_reason)
        return "stop"
    return mapped
