from typing import Final, Literal, Protocol, TypeAlias, TypedDict

from litellm.types.router import GenericLiteLLMParams

Headers: TypeAlias = dict[str, str]
Files: TypeAlias = list[tuple[str, bytes]]
Operation: TypeAlias = Literal["create_character", "get_character", "edit", "extension"]
API_BASE: Final[str] = "https://mock/videos"


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


# The inherited methods LiteLLM types with a bare dict or list, which hide their result types
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

    # ast-grep-ignore: no-long-names
    def transform_video_extension_request(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/videos/transformation.py#L384-L400
        self,
        prompt: str,
        video_id: str,
        seconds: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[str, Headers]: ...
