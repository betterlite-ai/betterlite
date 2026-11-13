from typing import Final, Literal, TypedDict

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.llms.base_llm.guardrail_translation.base_translation import StreamingScanKey
from litellm.proxy.guardrails.guardrail_hooks.unified_guardrail import unified_guardrail
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

from .chunks import Hooks

GUARDRAILS: Final = Hooks()


class StreamingFlagCall(TypedDict):
    function: Literal["streaming_flag"]
    attribute: Json | None
    guardrail_config: dict[str, Json] | None
    optional_params: dict[str, Json]
    name: str
    default: Json


class ScanKeyInput(TypedDict):
    texts: list[str]
    tool_calls: list[str]
    stream_ended: bool
    tool_calls_in_flight: bool


class RedundantScanCall(TypedDict):
    function: Literal["redundant_scan"]
    scan_key: ScanKeyInput | None
    last_scan_key: ScanKeyInput | None


def streaming_flag(call: StreamingFlagCall) -> Json:
    guardrail = (
        None
        if call["guardrail_config"] is None
        else CustomGuardrail(
            guardrail_name="b3679", guardrail_config=call["guardrail_config"]
        )
    )
    hooks = Hooks(**call["optional_params"])
    value = hooks.resolve_streaming_flag(guardrail, call["name"], call["default"])
    return {"value": TypeAdapter(Json).validate_python(value)}


def scan_key(fields: ScanKeyInput) -> StreamingScanKey:
    return StreamingScanKey(
        texts=tuple(fields["texts"]),
        tool_calls=tuple(fields["tool_calls"]),
        stream_ended=fields["stream_ended"],
        tool_calls_in_flight=fields["tool_calls_in_flight"],
    )


def redundant(call: RedundantScanCall) -> Json:
    key = None if call["scan_key"] is None else scan_key(call["scan_key"])
    last = None if call["last_scan_key"] is None else scan_key(call["last_scan_key"])
    return {"redundant": unified_guardrail._is_redundant_scan(key, last)}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L112
