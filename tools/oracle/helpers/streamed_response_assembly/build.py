from typing import Final, Literal, NotRequired, Required, TypedDict

from litellm.types.utils import Delta, FunctionCall, StreamingChoices, TextChoices
from pydantic import JsonValue as Json
from pydantic import TypeAdapter


class FunctionCallSpec(TypedDict):
    name: str
    arguments: str


class ChoiceSpec(TypedDict, total=False):
    index: Required[int]
    delta: Json
    text: Json
    finish_reason: Json
    logprobs: Json


class ChunkSpec(TypedDict, total=False):
    id: Json
    object: Json
    created: Json
    model: Json
    system_fingerprint: Json
    service_tier: Json
    usage: Json
    _hidden_params: Json
    choices: list[ChoiceSpec]


class LoggingInput(TypedDict):
    provider: str | None
    cache_hit: bool
    response_cost: NotRequired[Json]


class Input(TypedDict):
    function: Literal["assemble"]
    chunks: NotRequired[list[ChunkSpec] | None]
    messages: NotRequired[list[dict[str, Json]] | None]
    count_prompt_tokens: NotRequired[int | Literal["raise"] | None]
    counter: NotRequired[Json]
    costs: NotRequired[Json]
    logging: NotRequired[LoggingInput | None]
    now: NotRequired[int]


INPUT: Final[TypeAdapter[Input]] = TypeAdapter(Input)
FUNCTION_CALL: Final[TypeAdapter[FunctionCallSpec]] = TypeAdapter(FunctionCallSpec)
DELTA: Final[TypeAdapter[dict[str, Json]]] = TypeAdapter(dict[str, Json])

ChoiceWire = TextChoices | StreamingChoices | dict[str, Json]
ChunkWire = dict[str, Json | list[ChoiceWire]]


def build_chunk(spec: ChunkSpec) -> ChunkWire:
    chunk: ChunkWire = {}
    if "id" in spec:
        chunk["id"] = spec["id"]
    if "object" in spec:
        chunk["object"] = spec["object"]
    if "created" in spec:
        chunk["created"] = spec["created"]
    if "model" in spec:
        chunk["model"] = spec["model"]
    if "system_fingerprint" in spec:
        chunk["system_fingerprint"] = spec["system_fingerprint"]
    if "service_tier" in spec:
        chunk["service_tier"] = spec["service_tier"]
    if "usage" in spec:
        chunk["usage"] = spec["usage"]
    if "_hidden_params" in spec:
        chunk["_hidden_params"] = spec["_hidden_params"]
    choices = spec.get("choices")
    if choices is not None:
        chunk["choices"] = [build_choice(choice) for choice in choices]
    return chunk


def build_choice(spec: ChoiceSpec) -> ChoiceWire:
    if "text" in spec:
        return TextChoices(
            index=spec["index"],
            text=spec["text"],
            finish_reason=spec.get("finish_reason"),
            logprobs=spec.get("logprobs"),
        )
    delta = spec.get("delta")
    match delta:
        case dict():
            return dict_choice(spec, DELTA.validate_python(delta))
        case list() | str() | bool() | int() | float() | None:
            return mapping_choice(spec, delta)


# Only a function_call mapping becomes a LiteLLM record, since the join reads its attributes.
def dict_choice(spec: ChoiceSpec, delta: dict[str, Json]) -> ChoiceWire:
    call = delta.get("function_call")
    match call:
        case dict():
            return function_choice(spec, FUNCTION_CALL.validate_python(call))
        case list() | str() | bool() | int() | float() | None:
            return mapping_choice(spec, delta)


def function_choice(spec: ChoiceSpec, call: FunctionCallSpec) -> StreamingChoices:
    return StreamingChoices(
        index=spec["index"],
        delta=Delta(function_call=FunctionCall(**call)),
        finish_reason=spec.get("finish_reason"),
        logprobs=spec.get("logprobs"),
    )


def mapping_choice(spec: ChoiceSpec, delta: Json) -> dict[str, Json]:
    choice: dict[str, Json] = {"index": spec["index"], "delta": delta}
    finish_reason = spec.get("finish_reason")
    if finish_reason is not None:
        choice["finish_reason"] = finish_reason
    logprobs = spec.get("logprobs")
    if logprobs is not None:
        choice["logprobs"] = logprobs
    return choice
