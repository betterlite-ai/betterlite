from typing import Final, Literal, NotRequired, Required, TypedDict

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
