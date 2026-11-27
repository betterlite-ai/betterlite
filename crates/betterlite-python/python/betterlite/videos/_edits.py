from collections.abc import Mapping
from typing import Final

from pydantic import TypeAdapter

from betterlite._endpoints import CallValue, acall_native, call_native
from betterlite._native import NativeVideoEditError, NativeVideoEditSession
from betterlite.types.videos.main import CharacterObject, VideoObject
from betterlite.videos._calls import AsyncNativeCall, SyncNativeCall, video_payload

CHARACTER: Final[TypeAdapter[CharacterObject]] = TypeAdapter(CharacterObject)
EDITED: Final[TypeAdapter[VideoObject]] = TypeAdapter(VideoObject)
CREATE_NAMES: Final[tuple[str, ...]] = (
    "name", "video", "timeout", "custom_llm_provider", "extra_headers", "extra_query",
    "extra_body",
)
CHARACTER_NAMES: Final[tuple[str, ...]] = (
    "character_id", "timeout", "custom_llm_provider", "extra_headers", "extra_query",
    "extra_body",
)
EDIT_NAMES: Final[tuple[str, ...]] = (
    "video_id", "prompt", "video", "timeout", "custom_llm_provider", "extra_headers",
    "extra_query", "extra_body",
)
EXTENSION_NAMES: Final[tuple[str, ...]] = (
    "video_id", "prompt", "seconds", "timeout", "custom_llm_provider", "extra_headers",
    "extra_query", "extra_body",
)


def character_object(
    method: SyncNativeCall,
    arguments: Mapping[str, CallValue],
) -> CharacterObject:
    answer = call_native(method, video_payload(arguments), NativeVideoEditError)
    return CHARACTER.validate_python(answer)


async def acharacter_object(
    method: AsyncNativeCall,
    arguments: Mapping[str, CallValue],
) -> CharacterObject:
    answer = await acall_native(method, video_payload(arguments), NativeVideoEditError)
    return CHARACTER.validate_python(answer)


def edited_object(
    method: SyncNativeCall,
    arguments: Mapping[str, CallValue],
) -> VideoObject:
    answer = call_native(method, video_payload(arguments), NativeVideoEditError)
    return EDITED.validate_python(answer)


async def aedited_object(
    method: AsyncNativeCall,
    arguments: Mapping[str, CallValue],
) -> VideoObject:
    answer = await acall_native(method, video_payload(arguments), NativeVideoEditError)
    return EDITED.validate_python(answer)


def video_create_character(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L1141
    name: CallValue, video: CallValue, timeout: CallValue = 600,
    custom_llm_provider: CallValue = None, extra_headers: CallValue = None,
    extra_query: CallValue = None, extra_body: CallValue = None, **kwargs: CallValue,
) -> CharacterObject:
    arguments: dict[str, CallValue] = {
        "name": name, "video": video, "timeout": timeout,
        "custom_llm_provider": custom_llm_provider, "extra_headers": extra_headers,
        "extra_query": extra_query, "extra_body": extra_body, **kwargs,
    }
    return character_object(NativeVideoEditSession("{}").create_character, arguments)


async def avideo_create_character(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L1086
    name: CallValue, video: CallValue, timeout: CallValue = 600,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L1086
    custom_llm_provider: CallValue = None, extra_headers: CallValue = None,
    extra_query: CallValue = None, extra_body: CallValue = None, **kwargs: CallValue,
) -> CharacterObject:
    values = (name, video, timeout, custom_llm_provider, extra_headers, extra_query, extra_body)
    named: dict[str, CallValue] = {**kwargs, **dict(zip(CREATE_NAMES, values, strict=False))}
    return await acharacter_object(NativeVideoEditSession("{}").acreate_character, named)


def video_get_character(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L1267
    character_id: CallValue, timeout: CallValue = 600,
    custom_llm_provider: CallValue = None, extra_headers: CallValue = None,
    extra_query: CallValue = None, extra_body: CallValue = None, **kwargs: CallValue,
) -> CharacterObject:
    return character_object(NativeVideoEditSession("{}").get_character,
                            {**kwargs, **dict(zip(CHARACTER_NAMES, (
                                character_id, timeout, custom_llm_provider, extra_headers,
                                extra_query, extra_body), strict=False))})


async def avideo_get_character(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L1217
    character_id: CallValue, timeout: CallValue = 600,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L1217
    custom_llm_provider: CallValue = None, extra_headers: CallValue = None,
    extra_query: CallValue = None, extra_body: CallValue = None, **kwargs: CallValue,
) -> CharacterObject:
    named: dict[str, CallValue] = {
        "character_id": character_id, "timeout": timeout,
        "custom_llm_provider": custom_llm_provider, "extra_headers": extra_headers,
        "extra_query": extra_query, "extra_body": extra_body, **kwargs,
    }
    answer = await acall_native(NativeVideoEditSession("{}").aget_character,
                                video_payload(named), NativeVideoEditError)
    return CHARACTER.validate_python(answer)


def video_edit(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L1396
    video_id: CallValue, prompt: CallValue, timeout: CallValue = 600,
    custom_llm_provider: CallValue = None, extra_headers: CallValue = None,
    extra_query: CallValue = None, extra_body: CallValue = None,
    *, video: CallValue = None, **kwargs: CallValue,
) -> VideoObject:
    session = NativeVideoEditSession("{}")
    payload = video_payload({**kwargs, **dict(zip(EDIT_NAMES, (
        video_id, prompt, video, timeout, custom_llm_provider, extra_headers, extra_query,
        extra_body), strict=False))})
    answer = call_native(session.edit, payload, NativeVideoEditError)
    return EDITED.validate_python(answer)


async def avideo_edit(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L1341
    video_id: CallValue, prompt: CallValue, timeout: CallValue = 600,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L1341
    custom_llm_provider: CallValue = None, extra_headers: CallValue = None,
    extra_query: CallValue = None, extra_body: CallValue = None,
    *, video: CallValue = None, **kwargs: CallValue,
) -> VideoObject:
    session = NativeVideoEditSession("{}")
    named: dict[str, CallValue] = {
        "video_id": video_id, "prompt": prompt, "video": video,
        "timeout": timeout, "custom_llm_provider": custom_llm_provider,
        "extra_headers": extra_headers, "extra_query": extra_query, "extra_body": extra_body,
    }
    named |= kwargs
    return await aedited_object(session.aedit, named)


def video_extension(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L1531
    video_id: CallValue, prompt: CallValue, seconds: CallValue,
    timeout: CallValue = 600, custom_llm_provider: CallValue = None,
    extra_headers: CallValue = None, extra_query: CallValue = None,
    extra_body: CallValue = None, **kwargs: CallValue,
) -> VideoObject:
    arguments: dict[str, CallValue] = dict(zip(EXTENSION_NAMES, (
        video_id, prompt, seconds, timeout, custom_llm_provider, extra_headers, extra_query,
        extra_body), strict=False))
    arguments.update(kwargs)
    return edited_object(NativeVideoEditSession("{}").extension, arguments)


async def avideo_extension(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L1477
    video_id: CallValue, prompt: CallValue, seconds: CallValue,
    timeout: CallValue = 600,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L1477
    custom_llm_provider: CallValue = None, extra_headers: CallValue = None,
    extra_query: CallValue = None, extra_body: CallValue = None, **kwargs: CallValue,
) -> VideoObject:
    named: dict[str, CallValue] = {**kwargs}
    named["video_id"] = video_id
    named["prompt"] = prompt
    named["seconds"] = seconds
    named["timeout"] = timeout
    named["custom_llm_provider"] = custom_llm_provider
    named["extra_headers"] = extra_headers
    named["extra_query"] = extra_query
    named["extra_body"] = extra_body
    return await aedited_object(NativeVideoEditSession("{}").aextension, named)
