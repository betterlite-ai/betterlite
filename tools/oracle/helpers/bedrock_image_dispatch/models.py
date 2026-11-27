from dataclasses import dataclass, field
from typing import Final, Literal, Protocol, TypedDict, Unpack, runtime_checkable

from litellm.llms.bedrock.image_edit.handler import BedrockImageEdit
from litellm.llms.bedrock.image_generation.image_handler import BedrockImageGeneration
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.utils import ImageResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter


class Input(TypedDict):
    case: Literal[
        "titan_image",
        "stability_image",
        "canvas_image",
        "canvas_edit",
        "stability_edit",
    ]
    model_cost: dict[str, JsonValue]
    probes: list[dict[str, JsonValue]]


@dataclass
class Reply:
    status: int = 200
    headers: dict[str, str] = field(default_factory=dict)
    body: str = '{"images": ["eA=="]}'


@dataclass
class Probe:
    model: str
    prompt: str | None = None
    images: list[str] = field(default_factory=list)
    optional_params: dict[str, JsonValue] = field(default_factory=dict)
    extra_headers: dict[str, str] | None = None
    reply: Reply = field(default_factory=Reply)


PROBE: Final[TypeAdapter[Probe]] = TypeAdapter(Probe)


@runtime_checkable
class Logger(Protocol):
    def pre_call(self, **values: JsonValue) -> None: ...

    def post_call(self, **values: JsonValue) -> None: ...


class Recorder(Logger):
    def __init__(self) -> None:
        self.pre: dict[str, JsonValue] | None = None
        self.post: dict[str, JsonValue] | None = None

    def pre_call(self, **values: JsonValue) -> None:
        self.pre = values

    def post_call(self, **values: JsonValue) -> None:
        self.post = values


class GenerationArguments(TypedDict):
    model: str
    prompt: str
    model_response: ImageResponse
    optional_params: dict[str, JsonValue]
    logging_obj: Logger
    timeout: float | None
    aimg_generation: bool
    api_base: str | None
    extra_headers: dict[str, str] | None
    client: HTTPHandler | None
    api_key: str | None


class EditArguments(TypedDict):
    model: str
    image: list[str]
    prompt: str | None
    model_response: ImageResponse
    optional_params: dict[str, JsonValue]
    logging_obj: Logger
    timeout: float | None
    aimage_edit: bool
    api_base: str | None
    extra_headers: dict[str, str] | None
    client: HTTPHandler | None
    api_key: str | None


@runtime_checkable
class GenerationHandler(Protocol):
    def image_generation(self, **kwargs: Unpack[GenerationArguments]) -> ImageResponse: ...


@runtime_checkable
class EditHandler(Protocol):
    def image_edit(self, **kwargs: Unpack[EditArguments]) -> ImageResponse: ...


class GenerationBinding(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    handler: GenerationHandler


class EditBinding(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    handler: EditHandler


def generation_handler() -> GenerationHandler:
    values = {"handler": BedrockImageGeneration()}
    binding = GenerationBinding.model_validate(values)
    return binding.handler


def edit_handler() -> EditHandler:
    values = {"handler": BedrockImageEdit()}
    binding = EditBinding.model_validate(values)
    return binding.handler
