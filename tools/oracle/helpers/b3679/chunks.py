from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.proxy.guardrails.guardrail_hooks.unified_guardrail.unified_guardrail import (
    UnifiedLLMGuardrails,
)
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from starlette.exceptions import HTTPException as RaisedError

StreamItem: TypeAlias = ModelResponseStream | Json
JSON: Final[TypeAdapter[Json]] = TypeAdapter(Json)


class TransformArgs(TypedDict):
    mutated: dict[int, str]
    emitted: dict[int, str]
    holdback: dict[int, int]
    reasons: dict[int, str | None]
    held: dict[int, int]
    is_final: bool


class Hooks(UnifiedLLMGuardrails):
    def __init__(self, **optional: Json) -> None:
        self.optional_params = optional

    def carries_text(self, item: StreamItem) -> bool:
        return self._chunk_carries_text(item)

    def has_tool_calls(self, item: StreamItem) -> bool:
        return self._chunk_has_tool_calls(item)

    def has_finish_reason(self, item: StreamItem) -> bool:
        return self._chunk_has_finish_reason(item)

    def is_trailing_metadata(self, item: StreamItem) -> bool:
        return self._is_trailing_metadata_chunk(item)

    def trailing_indexes(self, items: list[StreamItem]) -> list[int]:
        kept = {id(item) for item in self._trailing_metadata_chunks(items)}
        return [index for index, item in enumerate(items) if id(item) in kept]

    def passthrough(
        self,
        item: StreamItem,
        reasons: dict[int, str | None] | None,
        held: frozenset[int],
    ) -> ModelResponseStream:
        return self._tool_call_passthrough_chunk(item, reasons, held)

    def transform(
        self, reference: ModelResponseStream, args: TransformArgs
    ) -> ModelResponseStream | None:
        return self._build_transform_chunk(
            reference_chunk=reference,
            mutated_text_per_choice=args["mutated"],
            emitted_text_per_choice=args["emitted"],
            holdback_per_choice=args["holdback"],
            finish_reason_per_choice=args["reasons"],
            held_chars_per_choice=args["held"],
            is_final=args["is_final"],
        )

    def streaming_flag(self, guardrail: CustomGuardrail, name: str, default: Json) -> Json:
        return JSON.validate_python(self.resolve_streaming_flag(guardrail, name, default))


HOOKS: Final = Hooks()


class ChunkInput(TypedDict):
    id: str
    created: int
    model: NotRequired[str | None]
    object: NotRequired[str]
    choices: list[Json]
    usage: NotRequired[Json]


class DeltaDump(TypedDict):
    content: NotRequired[Json]
    role: NotRequired[Json]
    tool_calls: NotRequired[Json]


class ChoiceDump(TypedDict):
    index: int
    finish_reason: NotRequired[Json]
    delta: DeltaDump


class ChunkDump(TypedDict):
    id: Json
    created: Json
    model: NotRequired[Json]
    object: str
    choices: list[ChoiceDump]


CHUNK: Final[TypeAdapter[ModelResponseStream]] = TypeAdapter(ModelResponseStream)
DUMP: Final[TypeAdapter[ChunkDump]] = TypeAdapter(ChunkDump)


def stream_item(value: Json) -> StreamItem:
    match value:
        case dict() as fields:
            return CHUNK.validate_python(fields)
        case None | bool() | int() | float() | str() | list():
            return value


def normalize(chunk: ModelResponseStream) -> Json:
    dump = DUMP.validate_json(chunk.model_dump_json())
    choices: list[Json] = [
        {
            "index": choice["index"],
            "finish_reason": choice.get("finish_reason"),
            "delta": {
                "content": choice["delta"].get("content"),
                "role": choice["delta"].get("role"),
                "tool_calls": choice["delta"].get("tool_calls"),
            },
        }
        for choice in dump["choices"]
    ]
    return {
        "id": dump["id"],
        "created": dump["created"],
        "model": dump.get("model"),
        "object": dump["object"],
        "choices": choices,
    }


def error_tag(error: RaisedError) -> Json:
    return {
        "error": {
            "status_code": error.status_code,
            "detail": JSON.validate_python(error.detail),
        }
    }


class PredicateCall(TypedDict):
    function: Literal["chunk_predicates"]
    items: list[Json]


def predicates(call: PredicateCall) -> Json:
    items = [stream_item(value) for value in call["items"]]
    carries_text: list[Json] = [HOOKS.carries_text(item) for item in items]
    has_tool_calls: list[Json] = [HOOKS.has_tool_calls(item) for item in items]
    has_finish_reason: list[Json] = [HOOKS.has_finish_reason(item) for item in items]
    is_trailing: list[Json] = [HOOKS.is_trailing_metadata(item) for item in items]
    trailing: list[Json] = [*HOOKS.trailing_indexes(items)]
    return {
        "carries_text": carries_text,
        "has_tool_calls": has_tool_calls,
        "has_finish_reason": has_finish_reason,
        "is_trailing_metadata": is_trailing,
        "trailing": trailing,
    }


class PassthroughCall(TypedDict):
    function: Literal["tool_call_passthrough"]
    item: Json
    reasons: list[tuple[int, str | None]] | None
    held: list[int]


def passthrough(call: PassthroughCall) -> Json:
    item = stream_item(call["item"])
    reasons = None if call["reasons"] is None else dict(call["reasons"])
    chunk = HOOKS.passthrough(item, reasons, frozenset(call["held"]))
    output_reasons: Json = (
        None if reasons is None else [[index, reason] for index, reason in reasons.items()]
    )
    return {"chunk": normalize(chunk), "reasons": output_reasons}


class TransformCall(TypedDict):
    function: Literal["transform_chunk"]
    reference_chunk: Json
    mutated: list[tuple[int, str]]
    emitted: list[tuple[int, str]]
    holdback: list[tuple[int, int]]
    reasons: list[tuple[int, str | None]]
    held: list[tuple[int, int]]
    is_final: bool


def transform_chunk(call: TransformCall) -> Json:
    emitted = dict(call["emitted"])
    held = dict(call["held"])
    args = TransformArgs(
        mutated=dict(call["mutated"]),
        emitted=emitted,
        holdback=dict(call["holdback"]),
        reasons=dict(call["reasons"]),
        held=held,
        is_final=call["is_final"],
    )
    try:
        chunk = HOOKS.transform(CHUNK.validate_python(call["reference_chunk"]), args)
    except RaisedError as error:
        return error_tag(error)
    return {
        "chunk": None if chunk is None else normalize(chunk),
        "emitted": [[index, text] for index, text in emitted.items()],
        "held": [[index, count] for index, count in held.items()],
    }
