from typing import NotRequired, Protocol, TypeAlias, TypedDict

import httpx
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.llms.openai import (  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L59
    ResponseInputParam,
)
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue

Object: TypeAlias = dict[str, JsonValue]
Input: TypeAlias = str | ResponseInputParam


class RouteCase(TypedDict):
    model: str | None


class AuthCase(TypedDict):
    api_key: NotRequired[str]
    headers: NotRequired[dict[str, str]]
    environment: NotRequired[dict[str, str]]
    sign: NotRequired[bool]


class ErrorProbe(TypedDict):
    status: int
    message: str
    headers: dict[str, str]


class SigningCase(TypedDict):
    headers: dict[str, str]
    optional_params: Object
    request_data: Object
    error: NotRequired[ErrorProbe]


class OptionCase(TypedDict):
    params: Object
    drop_params: NotRequired[bool]


class InputCase(TypedDict):
    input: Input


class ReplacementCase(InputCase):
    inlined: dict[str, str]


class FetchCase(InputCase):
    data_urls: dict[str, str]


class CodexCase(InputCase):
    model: str
    params: NotRequired[Object]


class UrlCase(TypedDict):
    api_base: NotRequired[str]
    litellm_params: NotRequired[Object]
    environment: NotRequired[dict[str, str]]


class Calls(Protocol):
    def validate_environment(
        self, headers: dict[str, str], model: str, litellm_params: GenericLiteLLMParams | None, /
    ) -> dict[str, str]: ...

    def sign_request(
        self,
        headers: dict[str, str],
        optional_params: Object,
        request_data: Object,
        api_base: str,
        /,
        *,
        api_key: str | None = None,
    ) -> tuple[dict[str, str], bytes | None]: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: httpx.Headers, /
    ) -> BaseLLMException: ...

    def get_complete_url(self, api_base: str | None, litellm_params: Object, /) -> str: ...

    def transform_responses_api_request(
        self,
        model: str,
        source: Input,
        params: Object,
        litellm_params: GenericLiteLLMParams,
        headers: Object,
        /,
    ) -> Object: ...

    async def async_transform_responses_api_request(
        self,
        model: str,
        source: Input,
        params: Object,
        litellm_params: GenericLiteLLMParams,
        headers: Object,
        /,
    ) -> Object: ...


class OptionCalls(Protocol):
    def map_openai_params(
        self, params: Object, model: str, /, *, drop_params: bool
    ) -> Object: ...

    def supports_native_file_search(self) -> bool: ...
