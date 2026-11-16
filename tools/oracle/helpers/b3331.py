import asyncio
import datetime as dt
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.videos.transformation import BaseVideoConfig
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.router import GenericLiteLLMParams
from litellm.types.videos.main import (
    CharacterObject,
    VideoCreateOptionalRequestParams,
    VideoObject,
)
from pydantic import TypeAdapter
from typing_extensions import override

Headers: TypeAlias = dict[str, str]
API_BASE: Final = "https://mock/videos"
FILES: TypeAlias = list[tuple[str, bytes]]
HEADERS: TypeAdapter[Headers] = TypeAdapter(Headers)


class MultipartDefaultInput(TypedDict):
    function: Literal["multipart_default"]


class PrefetchDefaultInput(TypedDict):
    function: Literal["prefetch_default"]


class CompleteUrlInput(TypedDict):
    function: Literal["complete_url"]
    api_bases: list[str | None]


class AsyncContentFallbackInput(TypedDict):
    function: Literal["async_content_fallback"]
    body: str


class AsyncStatusFallbackInput(TypedDict):
    function: Literal["async_status_fallback"]
    body: str
    custom_llm_provider: str | None


class UnsupportedRequestInput(TypedDict):
    function: Literal["unsupported_request"]
    operation: Literal["create_character", "get_character", "edit", "extension"]


class UnsupportedResponseInput(TypedDict):
    function: Literal["unsupported_response"]
    operation: Literal["create_character", "get_character", "edit", "extension"]


class ErrorClassInput(TypedDict):
    function: Literal["error_class"]
    error_message: str
    status_code: int
    headers: Headers


Input: TypeAlias = (
    MultipartDefaultInput
    | PrefetchDefaultInput
    | CompleteUrlInput
    | AsyncContentFallbackInput
    | AsyncStatusFallbackInput
    | UnsupportedRequestInput
    | UnsupportedResponseInput
    | ErrorClassInput
)


class ValueOutput(TypedDict):
    value: bool | None


class UrlView(TypedDict):
    url: str | None
    error: str | None


class CompleteUrlOutput(TypedDict):
    urls: list[UrlView]


class ContentOutput(TypedDict):
    content: str


class StatusOutput(TypedDict):
    id: str
    status: str
    progress: int | None


class UnsupportedOutput(TypedDict):
    raised: bool
    message: str


class ErrorClassOutput(TypedDict):
    message: str
    status_code: int
    headers: Headers


Output: TypeAlias = (
    ValueOutput
    | CompleteUrlOutput
    | ContentOutput
    | StatusOutput
    | UnsupportedOutput
    | ErrorClassOutput
)


# Implements every abstract method, so the inherited bodies run on an instance
class Subject(BaseVideoConfig):
    def __init__(self) -> None:
        pass

    @override
    def get_supported_openai_params(self, model: str) -> list[str]:
        return []

    @override
    def map_openai_params(
        self,
        video_create_optional_params: VideoCreateOptionalRequestParams,
        model: str,
        drop_params: bool,
    ) -> Headers:
        return {}

    @override
    def validate_environment(
        self,
        headers: Headers,
        model: str,
        api_key: str | None = None,
        litellm_params: GenericLiteLLMParams | None = None,
    ) -> Headers:
        return {}

    @override
    def get_complete_url(self, model: str, api_base: str | None, litellm_params: Headers) -> str:
        return super().get_complete_url(model, api_base, litellm_params)

    @override
    def transform_video_create_request(
        self,
        model: str,
        prompt: str,
        api_base: str,
        video_create_optional_request_params: Headers,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[Headers, FILES, str]:
        return {}, [], api_base

    @override
    def transform_video_create_response(
        self,
        model: str,
        raw_response: httpx.Response,
        logging_obj: Logging,
        custom_llm_provider: str | None = None,
        request_data: Headers | None = None,
    ) -> VideoObject:
        return VideoObject(id=raw_response.text, object="video", status="queued")

    @override
    def transform_video_content_request(
        self,
        video_id: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
        variant: str | None = None,
    ) -> tuple[str, Headers]:
        return api_base, {}

    @override
    def transform_video_content_response(
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> bytes:
        return raw_response.content

    @override
    def transform_video_remix_request(
        self,
        video_id: str,
        prompt: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
        extra_body: dict[str, str] | None = None,
    ) -> tuple[str, Headers]:
        return api_base, {}

    @override
    def transform_video_remix_response(
        self,
        raw_response: httpx.Response,
        logging_obj: Logging,
        custom_llm_provider: str | None = None,
    ) -> VideoObject:
        return VideoObject(id=raw_response.text, object="video", status="queued")

    @override
    def transform_video_list_request(
        self,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
        after: str | None = None,
        limit: int | None = None,
        order: str | None = None,
        extra_query: dict[str, str] | None = None,
    ) -> tuple[str, Headers]:
        return api_base, {}

    @override
    def transform_video_list_response(
        self,
        raw_response: httpx.Response,
        logging_obj: Logging,
        custom_llm_provider: str | None = None,
    ) -> Headers:
        return {}

    @override
    def transform_video_status_retrieve_request(
        self,
        video_id: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[str, Headers]:
        return api_base, {}

    @override
    def transform_video_status_retrieve_response(
        self,
        raw_response: httpx.Response,
        logging_obj: Logging,
        custom_llm_provider: str | None = None,
        client: HTTPHandler | None = None,
    ) -> VideoObject:
        status = "none" if custom_llm_provider is None else custom_llm_provider
        progress = 0 if client is None else 1
        return VideoObject(
            id=raw_response.text, object="video", status=status, progress=progress
        )


# LiteLLM declares these returns with bare dict and list, which mypy reads as Any
class AnyReturns(Protocol):
    def get_video_edit_prefetch_params(
        self,
        video_id: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[str, Headers] | None: ...

    def transform_video_create_character_request(
        self,
        name: str,
        video: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[str, FILES]: ...

    def transform_video_get_character_request(
        self,
        character_id: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[str, Headers]: ...

    def transform_video_extension_request(
        self,
        prompt: str,
        video_id: str,
        seconds: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[str, Headers]: ...


class RaisedError(Protocol):
    message: str
    status_code: int
    headers: Headers | httpx.Headers | None


SUBJECT: Final = Subject()
UNTYPED: Final[AnyReturns] = SUBJECT
PARAMS: Final = GenericLiteLLMParams()
LOGGING: Final = Logging(
    model="b3331",
    messages=None,
    stream=False,
    call_type="avideo_status",
    start_time=dt.datetime.now(tz=dt.UTC),
    litellm_call_id="b3331",
    function_id="b3331",
)


def complete_url(input_value: CompleteUrlInput) -> CompleteUrlOutput:
    urls: list[UrlView] = []
    for api_base in input_value["api_bases"]:
        try:
            urls.append({"url": SUBJECT.get_complete_url("m", api_base, {}), "error": None})
        except ValueError as error:
            urls.append({"url": None, "error": str(error)})
    return {"urls": urls}


async def fetch_status(input_value: AsyncStatusFallbackInput) -> VideoObject:
    handler = AsyncHTTPHandler()
    return await SUBJECT.async_transform_video_status_retrieve_response(
        httpx.Response(200, content=input_value["body"].encode()),
        LOGGING,
        input_value["custom_llm_provider"],
        handler,
    )


def async_status_fallback(input_value: AsyncStatusFallbackInput) -> StatusOutput:
    video = asyncio.run(fetch_status(input_value))
    return {"id": video.id, "status": video.status, "progress": video.progress}


def unsupported_request(input_value: UnsupportedRequestInput) -> UnsupportedOutput:
    try:
        match input_value["operation"]:
            case "create_character":
                UNTYPED.transform_video_create_character_request(
                    "name", "video", API_BASE, PARAMS, {}
                )
            case "get_character":
                UNTYPED.transform_video_get_character_request("char_1", API_BASE, PARAMS, {})
            case "edit":
                SUBJECT.transform_video_edit_request("prompt", "video_1", API_BASE, PARAMS, {})
            case "extension":
                UNTYPED.transform_video_extension_request(
                    "prompt", "video_1", "4", API_BASE, PARAMS, {}
                )
    except NotImplementedError as error:
        return {"raised": True, "message": str(error)}
    return {"raised": False, "message": ""}


def unsupported_response(input_value: UnsupportedResponseInput) -> UnsupportedOutput:
    raw = httpx.Response(200)
    try:
        match input_value["operation"]:
            case "create_character":
                SUBJECT.transform_video_create_character_response(raw, LOGGING)
            case "get_character":
                SUBJECT.transform_video_get_character_response(raw, LOGGING)
            case "edit":
                SUBJECT.transform_video_edit_response(raw, LOGGING)
            case "extension":
                SUBJECT.transform_video_extension_response(raw, LOGGING)
    except NotImplementedError as error:
        return {"raised": True, "message": str(error)}
    return {"raised": False, "message": ""}


def error_class(input_value: ErrorClassInput) -> ErrorClassOutput:
    try:
        SUBJECT.get_error_class(
            input_value["error_message"], input_value["status_code"], input_value["headers"]
        )
    except BaseLLMException as error:
        raised: RaisedError = error
        return {
            "message": raised.message,
            "status_code": raised.status_code,
            "headers": HEADERS.validate_python(raised.headers),
        }
    return {"message": "", "status_code": 0, "headers": {}}


def run(input_value: Input) -> Output:
    match input_value["function"]:
        case "multipart_default":
            return {"value": SUBJECT.use_multipart_form_data()}
        case "prefetch_default":
            prefetch = UNTYPED.get_video_edit_prefetch_params("video_1", API_BASE, PARAMS, {})
            return {"value": None if prefetch is None else True}
        case "complete_url":
            return complete_url(input_value)
        case "async_content_fallback":
            content = asyncio.run(
                SUBJECT.async_transform_video_content_response(
                    httpx.Response(200, content=input_value["body"].encode()), LOGGING
                )
            )
            return {"content": content.decode()}
        case "async_status_fallback":
            return async_status_fallback(input_value)
        case "unsupported_request":
            return unsupported_request(input_value)
        case "unsupported_response":
            return unsupported_response(input_value)
        case "error_class":
            return error_class(input_value)
