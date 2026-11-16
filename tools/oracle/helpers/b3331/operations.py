import asyncio
from typing import Final, Protocol, cast

import httpx
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from pydantic import TypeAdapter

from .inputs import (
    API_BASE,
    AsyncContentFallbackInput,
    AsyncStatusFallbackInput,
    CompleteUrlInput,
    CompleteUrlOutput,
    ContentOutput,
    ErrorClassInput,
    ErrorClassOutput,
    Headers,
    StatusOutput,
    UnsupportedOutput,
    UnsupportedRequestInput,
    UnsupportedResponseInput,
    UrlView,
    ValueOutput,
)
from .subject import EDIT_OPS, EMPTY_HEADERS, LOGGING, PARAMS, SUBJECT


# LiteLLM's BaseLLMException fields this helper reads; its headers are a bare dict
class ErrorView(Protocol):
    message: str
    status_code: int
    headers: Headers


HEADERS: Final[TypeAdapter[Headers]] = TypeAdapter(Headers)


def prefetch_default() -> ValueOutput:
    prefetch = EDIT_OPS.get_video_edit_prefetch_params("video_1", API_BASE, PARAMS, EMPTY_HEADERS)
    return {"value": None if prefetch is None else True}


def complete_url(input_value: CompleteUrlInput) -> CompleteUrlOutput:
    urls: list[UrlView] = []
    for api_base in input_value["api_bases"]:
        try:
            url = SUBJECT.get_complete_url("m", api_base, EMPTY_HEADERS)
        except ValueError as error:
            urls.append({"url": None, "error": str(error)})
        else:
            urls.append({"url": url, "error": None})
    return {"urls": urls}


def async_content_fallback(input_value: AsyncContentFallbackInput) -> ContentOutput:
    raw = httpx.Response(200, content=input_value["body"].encode())
    content = asyncio.run(SUBJECT.async_transform_video_content_response(raw, LOGGING))
    return {"content": content.decode()}


def async_status_fallback(input_value: AsyncStatusFallbackInput) -> StatusOutput:
    raw = httpx.Response(200, content=input_value["body"].encode())
    handler = AsyncHTTPHandler()
    video = asyncio.run(
        SUBJECT.async_transform_video_status_retrieve_response(
            raw, LOGGING, input_value["custom_llm_provider"], handler
        )
    )
    return {"id": video.id, "status": video.status, "progress": video.progress}


def unsupported_request(input_value: UnsupportedRequestInput) -> UnsupportedOutput:
    try:
        match input_value["operation"]:
            case "create_character":
                _ = EDIT_OPS.transform_video_create_character_request(
                    "name", b"", API_BASE, PARAMS, EMPTY_HEADERS
                )
            case "get_character":
                _ = EDIT_OPS.transform_video_get_character_request(
                    "char_1", API_BASE, PARAMS, EMPTY_HEADERS
                )
            case "edit":
                _ = SUBJECT.transform_video_edit_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/videos/transformation.py#L353-L373
                    "prompt", "video_1", API_BASE, PARAMS, EMPTY_HEADERS
                )
            case "extension":
                _ = EDIT_OPS.transform_video_extension_request(
                    "prompt", "video_1", "4", API_BASE, PARAMS, EMPTY_HEADERS
                )
    except NotImplementedError as error:
        return {"raised": True, "message": str(error)}
    return {"raised": False, "message": ""}


def unsupported_response(input_value: UnsupportedResponseInput) -> UnsupportedOutput:
    raw = httpx.Response(200)
    try:
        match input_value["operation"]:
            case "create_character":
                _ = SUBJECT.transform_video_create_character_response(raw, LOGGING)
            case "get_character":
                _ = SUBJECT.transform_video_get_character_response(raw, LOGGING)
            case "edit":
                _ = SUBJECT.transform_video_edit_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/videos/transformation.py#L375-L382
                    raw, LOGGING
                )
            case "extension":
                _ = SUBJECT.transform_video_extension_response(raw, LOGGING)
    except NotImplementedError as error:
        return {"raised": True, "message": str(error)}
    return {"raised": False, "message": ""}


def error_class(input_value: ErrorClassInput) -> ErrorClassOutput:
    try:
        _ = SUBJECT.get_error_class(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/videos/transformation.py#L410-L417
            input_value["error_message"], input_value["status_code"], input_value["headers"]
        )
    except BaseLLMException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41-L65
        raised = cast(ErrorView, error)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41-L65
        return {
            "message": raised.message,
            "status_code": raised.status_code,
            "headers": HEADERS.validate_python(raised.headers),
        }
    return {"message": "", "status_code": 0, "headers": {}}
