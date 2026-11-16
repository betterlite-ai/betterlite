from collections.abc import Callable, Sized
from io import BytesIO
from typing import Final, Never, Protocol, runtime_checkable

import httpx
from litellm.llms.openrouter.image_edit.transformation import OpenRouterImageEditConfig
from litellm.llms.openrouter.image_generation.transformation import (
    OpenRouterImageGenerationConfig,
)
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageResponse
from pydantic import ConfigDict, TypeAdapter

from .values import Headers, Object

ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)


@runtime_checkable
class GenerationConfig(Protocol):
    get_supported_openai_params: Callable[[str], list[str]]
    get_complete_url: Callable[[str | None, str | None, str, Object, Object], str]
    validate_environment: Callable[
        [Headers, str, list[Never], Object, Object, str | None, str | None], Headers
    ]
    transform_image_generation_request: Callable[[str, str, Object, Object, Headers], Object]
    transform_image_generation_response: Callable[
        [str, httpx.Response, ImageResponse, None, Object, Object, Object, None],
        ImageResponse,
    ]

    def map_openai_params(
        self,
        non_default_params: Object,
        optional_params: Object,
        model: str,
        *,
        drop_params: bool,
    ) -> Object: ...


@runtime_checkable
class EditConfig(Protocol):
    get_supported_openai_params: Callable[[str], list[str]]
    get_complete_url: Callable[[str, str | None, Object], str]
    validate_environment: Callable[
        [Headers, str, str | None, Object | None, str | None], Headers
    ]
    transform_image_edit_request: Callable[
        [str, str | None, bytes | BytesIO | None, Object, GenericLiteLLMParams, Headers],
        tuple[Object, Sized],
    ]
    transform_image_edit_response: Callable[[str, httpx.Response, None], ImageResponse]

    def map_openai_params(self, params: Object, model: str, *, drop_params: bool) -> Object: ...


# LiteLLM types these parameters as bare dicts and an untyped messages list, so the protocols are the declared shape
GENERATION: Final[GenerationConfig] = TypeAdapter[GenerationConfig](
    GenerationConfig, config=ARBITRARY
).validate_python(OpenRouterImageGenerationConfig())
EDIT: Final[EditConfig] = TypeAdapter[EditConfig](
    EditConfig, config=ARBITRARY
).validate_python(OpenRouterImageEditConfig())
