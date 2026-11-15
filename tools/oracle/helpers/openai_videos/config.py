from collections.abc import Callable, Mapping, Sequence
from typing import IO, Final, Protocol, TypeAlias

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.openai.videos.transformation import OpenAIVideoConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.videos.main import VideoObject
from pydantic import JsonValue

API_BASE: Final = "https://mock.example/v1/videos"
MODEL: Final = "sora-2"
ENV_NAME: Final = "OPENAI_API_KEY"

Content: TypeAlias = IO[bytes] | bytes | str
StoredFiles: TypeAlias = (
    Content
    | tuple[str | None, Content]
    | tuple[str | None, Content, str | None]
    | tuple[str | None, Content, str | None, Mapping[str, str]]
)
Parts: TypeAlias = Mapping[str, StoredFiles] | Sequence[tuple[str, StoredFiles]]
CreateParams: TypeAlias = dict[str, JsonValue | bytes]
Params: TypeAlias = dict[str, JsonValue]
UrlParams: TypeAlias = tuple[str, Params]
CreateForm: TypeAlias = tuple[Params, Parts, str]


class VideoConfig(Protocol):
    @property
    def get_supported_openai_params(self) -> Callable[[str], list[JsonValue]]: ...

    @property
    def validate_environment(
        self,
    ) -> Callable[[dict[str, str], str, str | None, GenericLiteLLMParams], Params]: ...

    @property
    def get_complete_url(self) -> Callable[[str, str | None, Params], str]: ...

    @property
    def use_multipart_form_data(self) -> Callable[[], bool]: ...

    @property
    def transform_video_create_request(
        self,
    ) -> Callable[
        [str, str, str, CreateParams, GenericLiteLLMParams, dict[str, str]], CreateForm
    ]: ...

    @property
    def transform_video_create_response(
        self,
    ) -> Callable[[str, httpx.Response, Logging, str | None, Params | None], VideoObject]: ...

    @property
    def transform_video_content_request(
        self,
    ) -> Callable[[str, str, GenericLiteLLMParams, dict[str, str], str | None], UrlParams]: ...

    @property
    def transform_video_content_response(
        self,
    ) -> Callable[[httpx.Response, Logging], bytes]: ...

    @property
    def transform_video_remix_request(
        self,
    ) -> Callable[[str, str, str, GenericLiteLLMParams, dict[str, str], None], UrlParams]: ...

    @property
    def transform_video_remix_response(
        self,
    ) -> Callable[[httpx.Response, Logging, str | None], VideoObject]: ...

    @property
    def transform_video_list_request(
        self,
    ) -> Callable[
        [str, GenericLiteLLMParams, dict[str, str], str | None, int | None, str | None, None],
        UrlParams,
    ]: ...

    @property
    def transform_video_list_response(
        self,
    ) -> Callable[[httpx.Response, Logging, str | None], Mapping[str, JsonValue]]: ...

    @property
    def transform_video_delete_request(
        self,
    ) -> Callable[[str, str, GenericLiteLLMParams, dict[str, str]], UrlParams]: ...

    @property
    def transform_video_delete_response(
        self,
    ) -> Callable[[httpx.Response, Logging], VideoObject]: ...

    @property
    def transform_video_status_retrieve_request(
        self,
    ) -> Callable[[str, str, GenericLiteLLMParams, dict[str, str]], UrlParams]: ...

    @property
    def transform_video_status_retrieve_response(
        self,
    ) -> Callable[[httpx.Response, Logging, str | None, None], VideoObject]: ...

    @property
    def get_error_class(
        self,
    ) -> Callable[[str, int, dict[str, str]], BaseLLMException]: ...


class VideoConfigImpl(OpenAIVideoConfig):
    # OpenAIVideoConfig.__init__ carries no annotations; this subclass states the one mypy needs
    def __init__(self) -> None:
        pass


CONFIG: VideoConfig = VideoConfigImpl()
