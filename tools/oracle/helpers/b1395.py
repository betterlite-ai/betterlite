import os
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict, Unpack

import litellm
import litellm.main
from litellm.main import (
    google_batch_embeddings,
    vertex_embedding,
    vertex_multimodal_embedding,
)
from pydantic import JsonValue, TypeAdapter

# get_llm_provider prints the provider list to stdout unless this is set
litellm.suppress_debug_info = True

Arm: TypeAlias = Literal["embed_content", "multimodal", "text"]

PROVIDER: Final = "vertex_ai"
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class LiteLLMSettings(TypedDict, total=False):
    vertex_project: str
    vertex_location: str
    api_base: str


class HandlerArguments(TypedDict, total=False):
    vertex_project: str | None
    vertex_location: str | None
    vertex_credentials: str | None
    api_base: str | None
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    api_key: str | None
    timeout: float | None
    extra_headers: dict[str, str]


class Input(TypedDict):
    config: str
    environment: dict[str, str]
    model: str
    input: str
    optional_params: dict[str, JsonValue]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]
    extra_headers: dict[str, str]
    timeout_seconds: float
    litellm_params: dict[str, JsonValue]
    catalog: NotRequired[dict[str, JsonValue]]


class View(TypedDict):
    arm: Arm
    project: str | None
    location: str | None
    credentials: str | None
    api_base: str | None
    remaining: list[str]
    extra_headers: dict[str, str] | None
    api_key_set: bool
    timeout: int | None
    litellm_params_set: bool


SETTINGS: Final[TypeAdapter[LiteLLMSettings]] = TypeAdapter(LiteLLMSettings)
INPUT: Final[TypeAdapter[Input]] = TypeAdapter(Input)


class Reply:
    pass


class Recorder:
    def __init__(self, arm: Arm) -> None:
        self.arm: Arm = arm
        self.call: HandlerArguments | None = None

    def record(self, **kwargs: Unpack[HandlerArguments]) -> Reply:
        self.call = kwargs
        return Reply()


class InjectedParams:
    def __init__(self, params: dict[str, JsonValue]) -> None:
        self.params = params

    def __call__(self, **kwargs: JsonValue) -> dict[str, JsonValue]:
        del kwargs
        return self.params


def apply_config(config: str) -> None:
    root = OBJECT.validate_json(config)
    section = root.get("litellm_settings")
    settings: LiteLLMSettings = {} if section is None else SETTINGS.validate_python(section)
    litellm.vertex_project = settings.get("vertex_project")
    litellm.vertex_location = settings.get("vertex_location")
    litellm.api_base = settings.get("api_base")


def remaining(call: HandlerArguments) -> list[str]:
    params = call.get("optional_params")
    if params is None:
        return []
    return sorted(params)


def seconds(timeout: float | None) -> int | None:
    if timeout is None:
        return None
    return int(timeout)


def view(arm: Arm, call: HandlerArguments) -> View:
    return {
        "arm": arm,
        "project": call.get("vertex_project"),
        "location": call.get("vertex_location"),
        "credentials": call.get("vertex_credentials"),
        "api_base": call.get("api_base"),
        "remaining": remaining(call),
        "extra_headers": call.get("extra_headers"),
        "api_key_set": call.get("api_key") is not None,
        "timeout": seconds(call.get("timeout")),
        "litellm_params_set": call.get("litellm_params") is not None,
    }


def recorded(recorders: tuple[Recorder, ...]) -> tuple[Arm, HandlerArguments]:
    for recorder in recorders:
        call = recorder.call
        if call is not None:
            return recorder.arm, call
    raise ValueError("no Vertex embedding handler ran")


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    apply_config(parsed["config"])
    for name, value in parsed["environment"].items():
        os.environ[name] = value
    catalog = parsed.get("catalog")
    if catalog is not None:
        litellm.register_model(catalog)
    embed_content = Recorder("embed_content")
    multimodal = Recorder("multimodal")
    text = Recorder("text")
    recorders = (embed_content, multimodal, text)
    object.__setattr__(google_batch_embeddings, "batch_embeddings", embed_content.record)
    object.__setattr__(vertex_multimodal_embedding, "multimodal_embedding", multimodal.record)
    object.__setattr__(vertex_embedding, "embedding", text.record)
    object.__setattr__(
        litellm.main,
        "get_optional_params_embeddings",
        InjectedParams(dict(parsed["optional_params"])),
    )
    litellm.embedding(
        model=parsed["model"],
        input=parsed["input"],
        custom_llm_provider=PROVIDER,
        api_key=parsed.get("api_key"),
        api_base=parsed.get("api_base"),
        timeout=parsed["timeout_seconds"],
        extra_headers=parsed["extra_headers"],
    )
    arm, call = recorded(recorders)
    return JSON.validate_python(view(arm, call))
