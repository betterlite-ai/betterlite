import asyncio
from typing import Final

import httpx
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from pydantic import BaseModel

from .inputs import (
    API_BASE,
    AsyncContentFallbackInput,
    AsyncStatusFallbackInput,
    ContentOutput,
    ErrorClassInput,
    ErrorClassOutput,
    Headers,
    StatusOutput,
    UnsupportedOutput,
    UnsupportedRequestInput,
    UnsupportedResponseInput,
    ValueOutput,
)
from .subject import EDIT_OPS, EMPTY_HEADERS, LOGGING, PARAMS, SUBJECT

PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException


class ErrorView(BaseModel):
    message: str
    status_code: int
    headers: Headers


def prefetch_default() -> ValueOutput:
    prefetch = EDIT_OPS.get_video_edit_prefetch_params("video_1", API_BASE, PARAMS, EMPTY_HEADERS)
    return {"value": None if prefetch is None else True}


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
                _ = EDIT_OPS.transform_video_edit_request(
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
                _ = EDIT_OPS.transform_video_edit_response(raw, LOGGING)
            case "extension":
                _ = SUBJECT.transform_video_extension_response(raw, LOGGING)
    except NotImplementedError as error:
        return {"raised": True, "message": str(error)}
    return {"raised": False, "message": ""}


def error_class(input_value: ErrorClassInput) -> ErrorClassOutput:
    try:
        _ = EDIT_OPS.get_error_class(
            input_value["error_message"], input_value["status_code"], input_value["headers"]
        )
    except PROVIDER_ERROR as failure:
        view = ErrorView.model_validate(failure, from_attributes=True)
        return {
            "message": view.message,
            "status_code": view.status_code,
            "headers": view.headers,
        }
    return {"message": "", "status_code": 0, "headers": {}}
