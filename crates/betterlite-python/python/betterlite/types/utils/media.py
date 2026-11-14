import time
from collections.abc import Mapping, Sequence
from typing import TypeAlias

from pydantic import JsonValue, PrivateAttr
from pydantic.main import IncEx
from typing_extensions import override

from betterlite.types.utils.base import (
    DictJsonObject,
    InitValue,
    ModelValue,
    ResponseObject,
    dump_value,
    validated_value,
)

Selection: TypeAlias = IncEx


class ImageObject(DictJsonObject):
    b64_json: str | None = None
    url: str | None = None
    revised_prompt: str | None = None
    provider_specific_fields: dict[str, JsonValue] | None = None

    @override
    def __init__(
        self, b64_json: JsonValue = None, url: JsonValue = None,
        revised_prompt: JsonValue = None,
        provider_specific_fields: Mapping[str, JsonValue] | None = None,
        **_params: InitValue,
    ) -> None:
        super().__init__(b64_json=b64_json, url=url, revised_prompt=revised_prompt)
        if provider_specific_fields:
            self.provider_specific_fields = dict(provider_specific_fields)


class ImageUsageInputTokensDetails(ResponseObject):
    image_tokens: int
    text_tokens: int


class ImageUsage(ResponseObject):
    input_tokens: int
    input_tokens_details: ImageUsageInputTokensDetails
    output_tokens: int
    total_tokens: int


class ImageResponse(DictJsonObject):
    created: int
    # a provider names its own background, format, quality and size, and LiteLLM keeps that string
    background: str | None = None
    data: list[ImageObject]
    output_format: str | None = None
    quality: str | None = None
    size: str | None = None
    usage: ImageUsage | None = None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    @override
    def __init__(
        self, created: JsonValue = None, data: Sequence[ModelValue] | None = None,
        response_ms: float | None = None, usage: ModelValue = None,
        hidden_params: Mapping[str, JsonValue] | None = None,
        background: str | None = None, quality: str | None = None,
        output_format: str | None = None, size: str | None = None,
        **_params: InitValue,
    ) -> None:
        images: list[ImageObject] = (
            [] if data is None
            else [validated_value(image, ImageObject) for image in data]
        )
        if usage is None:
            selected_usage: InitValue = ImageUsage(
                input_tokens=0,
                input_tokens_details=ImageUsageInputTokensDetails(image_tokens=0, text_tokens=0),
                output_tokens=0,
                total_tokens=0,
            )
        else:
            selected_usage = dump_value(usage)
        super().__init__(
            created=int(time.time()) if created is None else created,
            data=images, usage=selected_usage,
        )
        # LiteLLM assigns these after the model is built, so a provider's own strings pass
        self.background = background
        self.quality = quality
        self.output_format = output_format
        self.size = size
        if hidden_params is not None:
            self._hidden_params = dict(hidden_params)
