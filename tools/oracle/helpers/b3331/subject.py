import datetime as dt
from typing import Final, Protocol

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.videos.transformation import BaseVideoConfig
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.router import GenericLiteLLMParams
from litellm.types.videos.main import VideoCreateOptionalRequestParams, VideoObject
from typing_extensions import override

from .inputs import Files, Headers


class Subject(BaseVideoConfig):
    def __init__(self) -> None:
        pass

    @override
    def get_supported_openai_params(self, model: str) -> list[str]:
        return []

    @override
    def map_openai_params(
        self,
        # ast-grep-ignore: no-long-names
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
        return headers

    @override
    def get_complete_url(self, model: str, api_base: str | None, litellm_params: Headers) -> str:
        return super().get_complete_url(model, api_base, litellm_params)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/videos/transformation.py#L79-L94

    @override
    def transform_video_create_request(
        self,
        model: str,
        prompt: str,
        api_base: str,
        # ast-grep-ignore: no-long-names
        video_create_optional_request_params: Headers,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[Headers, Files, str]:
        return {}, [], api_base

    @override
    # ast-grep-ignore: no-long-names
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
    # ast-grep-ignore: no-long-names
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
    # ast-grep-ignore: no-long-names
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
        # ast-grep-ignore: no-loose-types
        extra_body: dict[str, object] | None = None,
    ) -> tuple[str, Headers]:
        return api_base, {"prompt": prompt, "video_id": video_id}

    @override
    def transform_video_remix_response(
        self,
        raw_response: httpx.Response,
        logging_obj: Logging,
        custom_llm_provider: str | None = None,
    ) -> VideoObject:
        status = "none" if custom_llm_provider is None else custom_llm_provider
        return VideoObject(id=raw_response.text, object="video", status=status)

    @override
    def transform_video_list_request(
        self,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
        after: str | None = None,
        limit: int | None = None,
        order: str | None = None,
        # ast-grep-ignore: no-loose-types
        extra_query: dict[str, object] | None = None,
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
    def transform_video_delete_request(
        self,
        video_id: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[str, Headers]:
        return api_base, {"video_id": video_id}

    @override
    # ast-grep-ignore: no-long-names
    def transform_video_delete_response(
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> VideoObject:
        return VideoObject(id=raw_response.text, object="video", status="deleted")

    @override
    # ast-grep-ignore: no-long-names
    def transform_video_status_retrieve_request(
        self,
        video_id: str,
        api_base: str,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[str, Headers]:
        return f"{api_base}/{video_id}", {}

    @override
    # ast-grep-ignore: no-long-names
    def transform_video_status_retrieve_response(
        self,
        raw_response: httpx.Response,
        logging_obj: Logging,
        custom_llm_provider: str | None = None,
        client: HTTPHandler | None = None,
    ) -> VideoObject:
        status = "none" if custom_llm_provider is None else custom_llm_provider
        progress = 0 if client is None else 1
        return VideoObject(id=raw_response.text, object="video", status=status, progress=progress)


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


SUBJECT: Final[Subject] = Subject()
EDIT_OPS: Final[EditOps] = SUBJECT
PARAMS: Final = GenericLiteLLMParams()
EMPTY_HEADERS: Final[Headers] = {}
LOGGING: Final = Logging(
    model="b3331",
    messages=None,
    stream=False,
    call_type="avideo_status",
    start_time=dt.datetime(2024, 1, 1, tzinfo=dt.UTC),
    litellm_call_id="b3331",
    function_id="b3331",
)
