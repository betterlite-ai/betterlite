from collections.abc import Awaitable, Callable, Mapping
from typing import Final, TypeAlias

from pydantic import JsonValue, TypeAdapter

from betterlite._completion.errors import raise_native_error
from betterlite._endpoints import (
    CallPayload,
    CallValue,
    acall_native,
    call_native,
    read_settings,
    to_call_payload,
)
from betterlite._native import NativeVideoError, NativeVideoSession
from betterlite.types.videos.main import VideoObject

SETTINGS: Final[tuple[str, ...]] = ("api_key", "openai_key", "drop_params")
GENERATION_NAMES: Final[tuple[str, ...]] = (
    "prompt", "model", "input_reference", "seconds", "size", "user", "timeout",
    "custom_llm_provider", "extra_headers", "extra_query", "extra_body",
)
CONTENT_NAMES: Final[tuple[str, ...]] = (
    "video_id", "timeout", "custom_llm_provider", "variant", "extra_headers", "extra_query",
    "extra_body",
)
REMIX_NAMES: Final[tuple[str, ...]] = (
    "video_id", "prompt", "timeout", "custom_llm_provider", "extra_headers", "extra_query",
    "extra_body",
)
LIST_NAMES: Final[tuple[str, ...]] = (
    "after", "limit", "order", "timeout", "custom_llm_provider", "extra_headers", "extra_query",
    "extra_body",
)
ASYNC_LIST_NAMES: Final[tuple[str, ...]] = (
    "after", "limit", "order", "api_key", "timeout", "custom_llm_provider", "extra_headers",
    "extra_query", "extra_body",
)
STATUS_NAMES: Final[tuple[str, ...]] = (
    "video_id", "timeout", "custom_llm_provider", "extra_headers", "extra_query", "extra_body",
)
REFUSED_NAMES: Final[tuple[str, ...]] = ("client", "logger_fn", "shared_session")

OBJECT: Final[TypeAdapter[VideoObject]] = TypeAdapter(VideoObject)
LISTING: Final[TypeAdapter[list[VideoObject] | dict[str, JsonValue]]] = TypeAdapter(
    list[VideoObject] | dict[str, JsonValue])
SyncNativeCall: TypeAlias = Callable[[str, list[bytes]], str]
AsyncNativeCall: TypeAlias = Callable[[str, list[bytes]], Awaitable[str]]
SyncContentCall: TypeAlias = Callable[[str, list[bytes]], bytes]
AsyncContentCall: TypeAlias = Callable[[str, list[bytes]], Awaitable[bytes]]


def video_payload(values: Mapping[str, CallValue]) -> CallPayload:
    payload: dict[str, CallValue] = {"settings": read_settings(SETTINGS)}
    payload["arguments"] = {
        name: value for name, value in values.items()
        if value is not None and name not in REFUSED_NAMES
    }
    payload.update({name: values.get(name) for name in REFUSED_NAMES})
    return to_call_payload(payload)


def video_object(method: SyncNativeCall, arguments: Mapping[str, CallValue]) -> VideoObject:
    answer = call_native(method, video_payload(arguments), NativeVideoError)
    return OBJECT.validate_python(answer)


async def avideo_object(method: AsyncNativeCall, arguments: Mapping[str, CallValue]) -> VideoObject:
    answer = await acall_native(method, video_payload(arguments), NativeVideoError)
    return OBJECT.validate_python(answer)


def video_objects(
    method: SyncNativeCall,
    arguments: Mapping[str, CallValue],
) -> list[VideoObject] | dict[str, JsonValue]:
    answer = call_native(method, video_payload(arguments), NativeVideoError)
    return LISTING.validate_python(answer)


async def avideo_objects(
    method: AsyncNativeCall,
    arguments: Mapping[str, CallValue],
) -> list[VideoObject] | dict[str, JsonValue]:
    answer = await acall_native(method, video_payload(arguments), NativeVideoError)
    return LISTING.validate_python(answer)


def video_bytes(method: SyncContentCall, arguments: Mapping[str, CallValue]) -> bytes:
    payload = video_payload(arguments)
    try:
        return method(payload.text, payload.blobs)
    except NativeVideoError as failure:
        raise_native_error(failure)


async def avideo_bytes(method: AsyncContentCall, arguments: Mapping[str, CallValue]) -> bytes:
    payload = video_payload(arguments)
    try:
        return await method(payload.text, payload.blobs)
    except NativeVideoError as failure:
        raise_native_error(failure)


def video_generation(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L165
    prompt: CallValue,
    model: CallValue = None,
    input_reference: CallValue = None,
    seconds: CallValue = None,
    size: CallValue = None,
    user: CallValue = None,
    timeout: CallValue = 600,
    custom_llm_provider: CallValue = None,
    extra_headers: CallValue = None,
    extra_query: CallValue = None,
    extra_body: CallValue = None,
    **kwargs: CallValue,
) -> VideoObject:
    session = NativeVideoSession("{}")
    named: dict[str, CallValue] = {
        "prompt": prompt, "model": model, "input_reference": input_reference,
        "seconds": seconds, "size": size, "user": user, "timeout": timeout,
        "custom_llm_provider": custom_llm_provider, "extra_headers": extra_headers,
        "extra_query": extra_query, "extra_body": extra_body, **kwargs,
    }
    return video_object(session.generation, named)


async def avideo_generation(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L35
    prompt: CallValue,
    model: CallValue = None,
    input_reference: CallValue = None,
    seconds: CallValue = None,
    size: CallValue = None,
    user: CallValue = None,
    timeout: CallValue = 600,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L35
    custom_llm_provider: CallValue = None,
    extra_headers: CallValue = None,
    extra_query: CallValue = None,
    extra_body: CallValue = None,
    **kwargs: CallValue,
) -> VideoObject:
    payload = video_payload({**kwargs, **dict(zip(GENERATION_NAMES, (
        prompt, model, input_reference, seconds, size, user, timeout,
        custom_llm_provider, extra_headers, extra_query, extra_body), strict=False))})
    answer = await acall_native(NativeVideoSession("{}").ageneration, payload, NativeVideoError)
    return OBJECT.validate_python(answer)


def video_content(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L273
    video_id: CallValue, timeout: CallValue = None, custom_llm_provider: CallValue = None,
    variant: CallValue = None, extra_headers: CallValue = None, extra_query: CallValue = None,
    extra_body: CallValue = None, **kwargs: CallValue) -> bytes:
    return video_bytes(NativeVideoSession("{}").content, {**kwargs, **dict(zip(
        CONTENT_NAMES, (video_id, timeout, custom_llm_provider, variant, extra_headers,
                        extra_query, extra_body), strict=False))})


async def avideo_content(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L383
    video_id: CallValue, timeout: CallValue = None,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L383
    custom_llm_provider: CallValue = None, variant: CallValue = None,
    extra_headers: CallValue = None, extra_query: CallValue = None, extra_body: CallValue = None,
    **kwargs: CallValue) -> bytes:
    values = (video_id, timeout, custom_llm_provider, variant, extra_headers, extra_query,
              extra_body)
    named: dict[str, CallValue] = {**kwargs, **dict(zip(CONTENT_NAMES, values, strict=False))}
    return await avideo_bytes(NativeVideoSession("{}").acontent, named)


def video_remix(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L557
    video_id: CallValue,
    prompt: CallValue,
    timeout: CallValue = 600,
    custom_llm_provider: CallValue = None,
    extra_headers: CallValue = None,
    extra_query: CallValue = None,
    extra_body: CallValue = None,
    **kwargs: CallValue,
) -> VideoObject:
    named: dict[str, CallValue] = {
        "video_id": video_id, "prompt": prompt, "timeout": timeout,
        "custom_llm_provider": custom_llm_provider, "extra_headers": extra_headers,
        "extra_query": extra_query, "extra_body": extra_body, **kwargs,
    }
    payload = video_payload(named)
    answer = call_native(NativeVideoSession("{}").remix, payload, NativeVideoError)
    return OBJECT.validate_python(answer)


async def avideo_remix(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L454
    video_id: CallValue,
    prompt: CallValue,
    timeout: CallValue = 600,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L454
    custom_llm_provider: CallValue = None,
    extra_headers: CallValue = None,
    extra_query: CallValue = None,
    extra_body: CallValue = None,
    **kwargs: CallValue,
) -> VideoObject:
    return await avideo_object(NativeVideoSession("{}").aremix, {**kwargs, **dict(zip(
        REMIX_NAMES, (video_id, prompt, timeout, custom_llm_provider, extra_headers,
                      extra_query, extra_body), strict=False))})


def video_list(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L771
    after: CallValue = None,
    limit: CallValue = None,
    order: CallValue = None,
    timeout: CallValue = 600,
    custom_llm_provider: CallValue = None,
    extra_headers: CallValue = None,
    extra_query: CallValue = None,
    extra_body: CallValue = None,
    **kwargs: CallValue,
) -> list[VideoObject] | dict[str, JsonValue]:
    query: dict[str, CallValue] = {"after": after, "limit": limit, "order": order}
    connection: dict[str, CallValue] = {
        "timeout": timeout, "custom_llm_provider": custom_llm_provider,
        "extra_headers": extra_headers, "extra_query": extra_query, "extra_body": extra_body,
    }
    return video_objects(NativeVideoSession("{}").list, {**kwargs, **connection, **query})


async def avideo_list(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L656
    after: CallValue = None,
    limit: CallValue = None,
    order: CallValue = None,
    api_key: CallValue = None,
    timeout: CallValue = 600,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L656
    custom_llm_provider: CallValue = None,
    extra_headers: CallValue = None,
    extra_query: CallValue = None,
    extra_body: CallValue = None,
    **kwargs: CallValue,
) -> list[VideoObject] | dict[str, JsonValue]:
    named: dict[str, CallValue] = {
        "after": after, "limit": limit, "order": order, "api_key": api_key,
        "timeout": timeout, "custom_llm_provider": custom_llm_provider,
        "extra_headers": extra_headers, "extra_query": extra_query, "extra_body": extra_body,
    }
    named |= kwargs
    return await avideo_objects(NativeVideoSession("{}").alist, named)


def video_status(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L969
    video_id: CallValue, timeout: CallValue = 600, custom_llm_provider: CallValue = None,
    extra_headers: CallValue = None, extra_query: CallValue = None, extra_body: CallValue = None,
    **kwargs: CallValue) -> VideoObject:
    return video_object(NativeVideoSession("{}").status, {**kwargs, **dict(zip(
        STATUS_NAMES, (video_id, timeout, custom_llm_provider, extra_headers, extra_query,
                       extra_body), strict=False))})


async def avideo_status(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L870
    video_id: CallValue, timeout: CallValue = 600, custom_llm_provider: CallValue = None,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L870
    extra_headers: CallValue = None, extra_query: CallValue = None, extra_body: CallValue = None,
    **kwargs: CallValue) -> VideoObject:
    return await avideo_object(NativeVideoSession("{}").astatus, {**kwargs, **dict(zip(
        STATUS_NAMES, (video_id, timeout, custom_llm_provider, extra_headers, extra_query,
                       extra_body), strict=False))})
