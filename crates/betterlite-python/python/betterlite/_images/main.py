from collections.abc import Awaitable, Callable, Mapping, Sequence
from typing import Final, TypeAlias

from pydantic import JsonValue

from betterlite._endpoints import (
    CallPayload,
    CallValue,
    read_settings,
    to_call_payload,
)
from betterlite._images.errors import raise_image_error
from betterlite._native import NativeImageError, NativeImageSession
from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.media import ImageObject, ImageResponse, ImageUsage

SETTINGS: Final[tuple[str, ...]] = ("api_base", "api_key", "drop_params", "openai_key")
GENERATION_NAMES: Final[tuple[str, ...]] = (
    "prompt", "model", "n", "quality", "response_format", "size", "style", "user",
    "timeout", "api_key", "api_base", "api_version", "custom_llm_provider",
)
VARIATION_NAMES: Final[tuple[str, ...]] = (
    "image", "model", "n", "response_format", "size", "user",
)

SyncNativeCall: TypeAlias = Callable[[str, list[bytes]], str]
AsyncNativeCall: TypeAlias = Callable[[str, list[bytes]], Awaitable[str]]


class ImageAnswer(ResponseObject):
    created: int
    data: list[ImageObject]
    usage: ImageUsage | None = None
    hidden_params: dict[str, JsonValue]


ANSWER: Final[TypeAdapter[ImageAnswer]] = TypeAdapter(ImageAnswer)


def image_generation(  # noqa: PLR0913, PLR0917
    prompt: CallValue,
    model: CallValue = None,
    n: CallValue = None,
    quality: CallValue = None,
    response_format: CallValue = None,
    size: CallValue = None,
    style: CallValue = None,
    user: CallValue = None,
    timeout: CallValue = 600,
    api_key: CallValue = None,
    api_base: CallValue = None,
    api_version: CallValue = None,
    custom_llm_provider: CallValue = None,
    **kwargs: CallValue,
) -> ImageResponse:
    values = dict(kwargs)
    values |= {
        "prompt": prompt, "model": model, "n": n, "quality": quality,
        "response_format": response_format, "size": size, "style": style,
        "user": user, "timeout": timeout, "api_key": api_key, "api_base": api_base,
        "api_version": api_version, "custom_llm_provider": custom_llm_provider,
    }
    text = call_image(NativeImageSession("{}").generate, image_payload(values))
    return image_response(text)


async def aimage_generation(*args: CallValue, **kwargs: CallValue) -> ImageResponse:
    values = positional(GENERATION_NAMES, args, kwargs)
    text = await acall_image(NativeImageSession("{}").agenerate, image_payload(values))
    return image_response(text)


def image_variation(  # noqa: PLR0913, PLR0917
    image: CallValue,
    model: CallValue = "dall-e-2",
    n: CallValue = 1,
    response_format: CallValue = "url",
    size: CallValue = None,
    user: CallValue = None,
    **kwargs: CallValue,
) -> ImageResponse:
    values = dict(kwargs)
    values |= {
        "image": image, "model": model, "n": n, "response_format": response_format,
        "size": size, "user": user,
    }
    text = call_image(NativeImageSession("{}").variation, image_payload(values))
    return image_response(text)


async def aimage_variation(*args: CallValue, **kwargs: CallValue) -> ImageResponse:
    values = positional(VARIATION_NAMES, args, kwargs)
    text = await acall_image(NativeImageSession("{}").avariation, image_payload(values))
    return image_response(text)


def image_edit(  # noqa: PLR0913, PLR0917
    image: CallValue = None,
    prompt: CallValue = None,
    model: CallValue = None,
    mask: CallValue = None,
    n: CallValue = None,
    quality: CallValue = None,
    response_format: CallValue = None,
    size: CallValue = None,
    user: CallValue = None,
    extra_headers: CallValue = None,
    extra_query: CallValue = None,
    extra_body: CallValue = None,
    timeout: CallValue = None,
    custom_llm_provider: CallValue = None,
    **kwargs: CallValue,
) -> ImageResponse:
    values = dict(kwargs)
    values |= {
        "image": image, "prompt": prompt, "model": model, "mask": mask, "n": n,
        "quality": quality, "response_format": response_format, "size": size,
        "user": user, "extra_headers": extra_headers, "extra_query": extra_query,
        "extra_body": extra_body, "timeout": timeout,
        "custom_llm_provider": custom_llm_provider,
    }
    text = call_image(NativeImageSession("{}").edit, image_payload(values))
    return image_response(text)


async def aimage_edit(  # noqa: ASYNC109, PLR0913, PLR0917
    image: CallValue,
    model: CallValue,
    prompt: CallValue,
    mask: CallValue = None,
    n: CallValue = None,
    quality: CallValue = None,
    response_format: CallValue = None,
    size: CallValue = None,
    user: CallValue = None,
    extra_headers: CallValue = None,
    extra_query: CallValue = None,
    extra_body: CallValue = None,
    timeout: CallValue = None,
    custom_llm_provider: CallValue = None,
    **kwargs: CallValue,
) -> ImageResponse:
    values = dict(kwargs)
    values |= {
        "image": image, "prompt": prompt, "model": model, "mask": mask, "n": n,
        "quality": quality, "response_format": response_format, "size": size,
        "user": user, "extra_headers": extra_headers, "extra_query": extra_query,
        "extra_body": extra_body, "timeout": timeout,
        "custom_llm_provider": custom_llm_provider,
    }
    text = await acall_image(NativeImageSession("{}").aedit, image_payload(values))
    return image_response(text)


def positional(
    names: Sequence[str],
    args: tuple[CallValue, ...],
    kwargs: Mapping[str, CallValue],
) -> dict[str, CallValue]:
    values = dict(kwargs)
    for name, value in zip(names, args, strict=False):
        values[name] = value
    return values


def image_payload(values: Mapping[str, CallValue]) -> CallPayload:
    payload: dict[str, CallValue] = {
        "arguments": forwarded(values),
        "settings": read_settings(SETTINGS),
        "client": values.get("client"),
        "logger_fn": values.get("logger_fn"),
        "shared_session": values.get("shared_session"),
    }
    return to_call_payload(payload)


def forwarded(values: Mapping[str, CallValue]) -> dict[str, CallValue]:
    forwarded_values: dict[str, CallValue] = {}
    for name, value in values.items():
        if value is not None:
            forwarded_values[name] = value
    return forwarded_values


def call_image(method: SyncNativeCall, payload: CallPayload) -> str:
    try:
        return method(payload.text, payload.blobs)
    except NativeImageError as failure:
        raise_image_error(failure)


async def acall_image(method: AsyncNativeCall, payload: CallPayload) -> str:
    try:
        return await method(payload.text, payload.blobs)
    except NativeImageError as failure:
        raise_image_error(failure)


def image_response(text: str) -> ImageResponse:
    answer = ANSWER.validate_json(text)
    return ImageResponse(
        created=answer.created,
        data=answer.data,
        usage=answer.usage,
        hidden_params=answer.hidden_params,
    )


def __getattr__(name: str) -> type[object]:
    # LiteLLM's images.main loads ImageEditRequestUtils lazily; the edit rules run in Rust here
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
