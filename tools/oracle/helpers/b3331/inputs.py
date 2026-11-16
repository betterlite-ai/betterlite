from collections.abc import Callable, Mapping
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from httpx._types import RequestFiles
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.router import GenericLiteLLMParams
from litellm.types.videos.main import VideoObject
from pydantic import JsonValue

Headers: TypeAlias = dict[str, str]
Files: TypeAlias = list[tuple[str, bytes]]
Operation: TypeAlias = Literal["create_character", "get_character", "edit", "extension"]
API_BASE: Final[str] = "https://mock/videos"


class MultipartDefaultInput(TypedDict):
    function: Literal["multipart_default"]


class PrefetchDefaultInput(TypedDict):
    function: Literal["prefetch_default"]


class AsyncContentFallbackInput(TypedDict):
    function: Literal["async_content_fallback"]
    body: str


class AsyncStatusFallbackInput(TypedDict):
    function: Literal["async_status_fallback"]
    body: str
    custom_llm_provider: str | None


class UnsupportedRequestInput(TypedDict):
    function: Literal["unsupported_request"]
    operation: Operation


class UnsupportedResponseInput(TypedDict):
    function: Literal["unsupported_response"]
    operation: Operation


class ErrorClassInput(TypedDict):
    function: Literal["error_class"]
    error_message: str
    status_code: int
    headers: Headers


Input: TypeAlias = (
    MultipartDefaultInput
    | PrefetchDefaultInput
    | AsyncContentFallbackInput
    | AsyncStatusFallbackInput
    | UnsupportedRequestInput
    | UnsupportedResponseInput
    | ErrorClassInput
)


class ValueOutput(TypedDict):
    value: bool | None


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
    | ContentOutput
    | StatusOutput
    | UnsupportedOutput
    | ErrorClassOutput
)


# The inherited methods LiteLLM types with a bare dict, which hide their result types
class EditOps(Protocol):
    def get_video_edit_prefetch_params(
        self, video_id: str, api_base: str, litellm_params: GenericLiteLLMParams, headers: Headers
    ) -> tuple[str, Headers] | None: ...

    # ast-grep-ignore: no-long-names
    def transform_video_create_character_request(
        self,
        name: str,
        video: bytes,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[str, Files]: ...

    # ast-grep-ignore: no-long-names
    def transform_video_get_character_request(
        self,
        character_id: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[str, Headers]: ...

    def transform_video_edit_request(
        self,
        prompt: str,
        video_id: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[
        str,
        Mapping[str, object],  # ast-grep-ignore: no-loose-types
        RequestFiles | None,
    ]: ...

    def transform_video_edit_response(
        self,
        raw_response: httpx.Response,
        logging_obj: Logging,
        custom_llm_provider: str | None = None,
    ) -> VideoObject: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: Headers
    ) -> BaseLLMException: ...

    @property
    # ast-grep-ignore: no-long-names
    def transform_video_extension_request(
        self,
    ) -> Callable[
        [str, str, str, str, GenericLiteLLMParams, Headers], tuple[str, dict[str, JsonValue]]
    ]: ...
