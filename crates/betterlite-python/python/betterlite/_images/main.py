from collections.abc import Awaitable, Callable, Mapping
from typing import Final, NoReturn, TypeAlias

from pydantic import JsonValue, TypeAdapter

from betterlite._endpoints import (
    CallPayload,
    CallValue,
    model_from_arguments,
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
    background: str | None = None
    quality: str | None = None
    output_format: str | None = None
    size: str | None = None
    hidden_params: dict[str, JsonValue]


ANSWER: Final[TypeAdapter[ImageAnswer]] = TypeAdapter(ImageAnswer)


def image_generation(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L184
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
    given = (prompt, model, n, quality, response_format, size, style, user,
             timeout, api_key, api_base, api_version, custom_llm_provider)
    return call_image(
        NativeImageSession("{}").generate,
        {**kwargs, **dict(zip(GENERATION_NAMES, given, strict=False))},
    )


async def aimage_generation(*args: CallValue, **kwargs: CallValue) -> ImageResponse:
    _ = model_from_arguments(args, kwargs)
    arguments = {**kwargs, **dict(zip(GENERATION_NAMES, args, strict=False))}
    session = NativeImageSession("{}")
    return image_response(await acall_image(session.agenerate, image_payload(arguments)))


def image_variation(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L616
    image: CallValue,
    model: CallValue = "dall-e-2",
    n: CallValue = 1,
    response_format: CallValue = "url",
    size: CallValue = None,
    user: CallValue = None,
    **kwargs: CallValue,
) -> ImageResponse:
    named = dict(kwargs, image=image, model=model, n=n,
                 response_format=response_format, size=size, user=user)
    return call_image(NativeImageSession("{}").variation, named)


async def aimage_variation(*args: CallValue, **kwargs: CallValue) -> ImageResponse:
    return image_response(
        await acall_image(
            NativeImageSession("{}").avariation,
            image_payload({**kwargs, **dict(zip(VARIATION_NAMES, args, strict=False))}),
        )
    )


def image_edit(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L712
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
    named = dict(kwargs, image=image, prompt=prompt, model=model, mask=mask, n=n,
                 quality=quality, response_format=response_format, size=size, user=user,
                 extra_headers=extra_headers, extra_query=extra_query, extra_body=extra_body,
                 timeout=timeout, custom_llm_provider=custom_llm_provider)
    return call_image(NativeImageSession("{}").edit, named)


async def aimage_edit(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L964
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
    timeout: CallValue = None,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L964
    custom_llm_provider: CallValue = None,
    **kwargs: CallValue,
) -> ImageResponse:
    named = dict(kwargs, image=image, prompt=prompt, model=model, mask=mask, n=n,
                 quality=quality, response_format=response_format, size=size, user=user,
                 extra_headers=extra_headers, extra_query=extra_query, extra_body=extra_body,
                 timeout=timeout, custom_llm_provider=custom_llm_provider)
    return image_response(
        await acall_image(NativeImageSession("{}").aedit, image_payload(named))
    )


def image_payload(values: Mapping[str, CallValue]) -> CallPayload:
    payload: dict[str, CallValue] = {"settings": read_settings(SETTINGS)}
    payload["arguments"] = {
        name: value for name, value in values.items()
        if value is not None and name not in ("client", "logger_fn", "shared_session")
    }
    payload.update({name: values.get(name) for name in ("client", "logger_fn", "shared_session")})
    return to_call_payload(payload)


def call_image(method: SyncNativeCall, arguments: Mapping[str, CallValue]) -> ImageResponse:
    payload = image_payload(arguments)
    try:
        return image_response(method(payload.text, payload.blobs))
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
        background=answer.background,
        quality=answer.quality,
        output_format=answer.output_format,
        size=answer.size,
    )


def __getattr__(name: str) -> NoReturn:
    # LiteLLM's images.main loads ImageEditRequestUtils lazily; the edit rules run in Rust here
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
