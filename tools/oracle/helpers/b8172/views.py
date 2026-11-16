from collections.abc import Callable
from typing import Literal, NotRequired, Protocol, TypedDict, runtime_checkable

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ImageResponse
from pydantic import BaseModel, ConfigDict, JsonValue


class NoMessages(list[AllMessageValues]): ...


class Recorder(Logging):
    def __init__(self) -> None:
        self.model_call_details: dict[str, JsonValue] = {}


ImageReplyTransform = Callable[
    [
        str,
        httpx.Response,
        ImageResponse,
        Recorder,
        dict[str, JsonValue],
        dict[str, JsonValue],
        dict[str, JsonValue],
        None,
        str | None,
        bool | None,
    ],
    ImageResponse,
]

ImageEnvTransform = Callable[
    [
        dict[str, str],
        str,
        NoMessages,
        dict[str, JsonValue],
        dict[str, JsonValue],
        str | None,
    ],
    dict[str, JsonValue],
]

UrlTransform = Callable[
    [str | None, str | None, str, dict[str, JsonValue], dict[str, JsonValue]],
    str,
]

ImageRequestTransform = Callable[
    [str, str, dict[str, JsonValue], dict[str, JsonValue], dict[str, str]],
    dict[str, JsonValue],
]


@runtime_checkable
class ImageConfig(Protocol):
    def map_openai_params(
        self, non_default_params: dict[str, JsonValue], optional_params: dict[str, JsonValue],
        model: str, /, *, drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    # ruff counts a def's parameters (PLR0913), so each six-argument call type is a property value
    @property
    def validate_environment(self) -> ImageEnvTransform: ...

    @property
    def get_complete_url(self) -> UrlTransform: ...

    @property
    def transform_image_generation_request(  # ast-grep-ignore: no-long-names
        self,
    ) -> ImageRequestTransform: ...

    @property
    def transform_image_generation_response(  # ast-grep-ignore: no-long-names
        self,
    ) -> ImageReplyTransform: ...


@runtime_checkable
class EmbeddingUrlConfig(Protocol):
    @property
    def get_complete_url(self) -> UrlTransform: ...


@runtime_checkable
class RerankUrlConfig(Protocol):
    def get_complete_url(
        self, api_base: str | None, model: str, optional_params: dict[str, JsonValue] | None, /,
    ) -> str: ...


class ImageBoundary(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: ImageConfig


class EmbeddingBoundary(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: EmbeddingUrlConfig


class RerankBoundary(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: RerankUrlConfig


class ImageParamsAndSizeInput(TypedDict):
    function: Literal["image_params_and_size"]
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]


class ImageUrlAndKeyInput(TypedDict):
    function: Literal["image_url_and_key"]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    env_key: NotRequired[str | None]
    env_image_base: NotRequired[str | None]
    caller_headers: NotRequired[dict[str, str]]


class ImageRequestInput(TypedDict):
    function: Literal["image_request"]
    model: str
    prompt: str
    optional_params: dict[str, JsonValue]


class ImageReplyInput(TypedDict):
    function: Literal["image_reply"]
    body: str
    status: int
    headers: NotRequired[dict[str, str]]


class FamilyStep(TypedDict):
    base: NotRequired[str | None]
    brand_key: NotRequired[str | None]
    fallback_key: NotRequired[str | None]
    rerank_base: NotRequired[str | None]
    image_base: NotRequired[str | None]


class FamilySettingsInput(TypedDict):
    function: Literal["family_settings"]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    dashscope: FamilyStep
    qwencloud: FamilyStep
    qwen_ai_platform: FamilyStep


class FamilyKeyAndMessageInput(TypedDict):
    function: Literal["family_key_and_message"]
    api_key: NotRequired[str | None]
    dashscope: NotRequired[str | None]
    qwencloud: NotRequired[str | None]
    qwen_ai_platform: NotRequired[str | None]


class ImageDatum(TypedDict):
    url: str | None


class ImageReplyDump(TypedDict):
    data: list[ImageDatum]


Input = (
    ImageParamsAndSizeInput | ImageUrlAndKeyInput | ImageRequestInput
    | ImageReplyInput | FamilySettingsInput | FamilyKeyAndMessageInput
)
