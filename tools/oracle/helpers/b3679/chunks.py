from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy.guardrails.guardrail_hooks.unified_guardrail.unified_guardrail import (
    UnifiedLLMGuardrails,
)
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue as Json
from pydantic import TypeAdapter


class Hooks(UnifiedLLMGuardrails):
    def __init__(self, **optional: Json) -> None:
        super().__init__(**optional)  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L171

GUARDRAILS: Final = Hooks()


class DeltaDump(TypedDict):
    content: str | None
    role: str | None
    tool_calls: list[Json] | None


class ChoiceDump(TypedDict):
    index: int
    finish_reason: str | None
    delta: DeltaDump


class ChunkDump(TypedDict):
    id: str | None
    created: int | None
    model: str | None
    object: str
    choices: list[ChoiceDump]


class PredicateCall(TypedDict):
    function: Literal["chunk_predicates"]
    item: Json


class PassthroughCall(TypedDict):
    function: Literal["tool_call_passthrough"]
    item: Json
    reasons: list[tuple[int, str | None]] | None
    held: list[int]


StreamItem: TypeAlias = ModelResponseStream | Json


def stream_item(value: Json) -> StreamItem:
    match value:
        case dict() as fields:
            return ModelResponseStream(**fields)  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2110
        case None | bool() | int() | float() | str() | list():
            return value


def normalize(chunk: ModelResponseStream) -> Json:
    dump = TypeAdapter(ChunkDump).validate_json(chunk.model_dump_json())
    choices: list[Json] = [
        {
            "index": choice["index"],
            "finish_reason": choice["finish_reason"],
            "delta": {
                "content": choice["delta"]["content"],
                "role": choice["delta"]["role"],
                "tool_calls": choice["delta"].get("tool_calls"),
            },
        }
        for choice in dump["choices"]
    ]
    return {
        "id": dump.get("id"),
        "created": dump.get("created"),
        "model": dump.get("model"),
        "object": dump["object"],
        "choices": choices,
    }


def predicates(call: PredicateCall) -> Json:
    item = stream_item(call["item"])
    return {
        "carries_text": GUARDRAILS._chunk_carries_text(item),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L890
        "has_tool_calls": GUARDRAILS._chunk_has_tool_calls(item),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L865
        "has_finish_reason": GUARDRAILS._chunk_has_finish_reason(item),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L959
        "is_trailing_metadata": GUARDRAILS._is_trailing_metadata_chunk(item),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L873
    }


def passthrough(call: PassthroughCall) -> Json:
    item = stream_item(call["item"])
    reasons: dict[int, str | None] | None = None
    if call["reasons"] is not None:
        reasons = dict(call["reasons"])
    chunk = GUARDRAILS._tool_call_passthrough_chunk(item, reasons, frozenset(call["held"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L900
    output_reasons: Json = (
        None
        if reasons is None
        else [[index, reason] for index, reason in reasons.items()]
    )
    return {"chunk": normalize(chunk), "reasons": output_reasons}
