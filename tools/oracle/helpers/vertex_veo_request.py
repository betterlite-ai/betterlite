from collections.abc import Callable
from typing import Final, Protocol, TypedDict, runtime_checkable

import litellm
from litellm.llms.vertex_ai.videos.transformation import VertexAIVideoConfig
from litellm.types.router import GenericLiteLLMParams
from pydantic import ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JsonObject = dict[str, JsonValue]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ARBITRARY_TYPES: Final = ConfigDict(arbitrary_types_allowed=True)
DROP_PARAMS: Final[bool] = False
FIELDS: Final[tuple[str, ...]] = (
    "input_reference",
    "image",
    "parameters",
    "resolution",
    "seconds",
    "size",
)


@runtime_checkable
class VeoConfig(Protocol):
    map_openai_params: Callable[[dict[str, JsonValue], str, bool], dict[str, JsonValue]]
    transform_video_create_request: Callable[
        [str, str, str, dict[str, JsonValue], GenericLiteLLMParams, dict[str, str]],
        tuple[JsonObject, JsonValue, str],
    ]
    transform_video_edit_request: Callable[
        [
            str,
            str,
            str,
            GenericLiteLLMParams,
            dict[str, str],
            None,
            dict[str, JsonValue] | None,
            dict[str, JsonValue] | None,
        ],
        tuple[str, JsonObject, JsonValue],
    ]


# LiteLLM's __init__ is untyped, and every method this file calls is stateless
HANDLER: Final = VertexAIVideoConfig.__new__(VertexAIVideoConfig)
# litellm's config classes carry untyped parameters, so the protocol is the declared shape
CONFIG: Final[VeoConfig] = TypeAdapter[VeoConfig](
    VeoConfig, config=ARBITRARY_TYPES
).validate_python(HANDLER)


class CreateStep(TypedDict):
    model: str
    prompt: str
    api_base: str
    params: JsonObject
    extra_body: JsonObject | None


class EditStep(TypedDict):
    prompt: str
    api_base: str
    video_id: str
    extra_body: JsonObject | None
    prefetched: JsonObject | None


class Input(TypedDict):
    creates: list[CreateStep]
    edits: list[EditStep]


def optional_params(step: CreateStep) -> dict[str, JsonValue]:
    given: JsonObject = {}
    extra = step["extra_body"]
    for name in FIELDS:
        value = step["params"].get(name)
        if extra is not None:
            overridden = extra.get(name)
            if overridden is not None:
                value = overridden
        if value is not None:
            given[name] = value
    return given


def mapped_params(step: CreateStep) -> dict[str, JsonValue]:
    mapped = CONFIG.map_openai_params(optional_params(step), step["model"], DROP_PARAMS)
    extra = step["extra_body"]
    if extra is not None:
        for name, value in extra.items():
            if value is not None:
                mapped[name] = value
    if "extra_body" in mapped:
        mapped.pop("extra_body")
    return mapped


def request_answer(url: str, body: JsonObject) -> JsonValue:
    answer: JsonObject = {"url": url, "body": body}
    return JSON.validate_python(answer)


def run(input_value: Input) -> JsonValue:
    creates: list[JsonValue] = []
    for step in input_value["creates"]:
        body, _files, url = CONFIG.transform_video_create_request(
            step["model"],
            step["prompt"],
            step["api_base"],
            mapped_params(step),
            GenericLiteLLMParams(),
            {},
        )
        creates.append(request_answer(url, body))
    edits: list[JsonValue] = []
    for step in input_value["edits"]:
        url, body, _files = CONFIG.transform_video_edit_request(
            step["prompt"],
            step["video_id"],
            step["api_base"],
            GenericLiteLLMParams(),
            {},
            None,
            step["extra_body"],
            step["prefetched"],
        )
        edits.append(request_answer(url, body))
    answer: JsonObject = {"creates": creates, "edits": edits}
    return JSON.validate_python(answer)
