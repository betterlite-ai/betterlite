import os
from collections.abc import Iterator, Mapping
from contextlib import contextmanager
from typing import Final, Protocol

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.containers.main import (
    ContainerCreateOptionalRequestParams as CreateParams,
    ContainerFileListResponse,
    ContainerListResponse,
    ContainerObject,
    DeleteContainerResult,
)
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue, TypeAdapter

from .cases import Globals

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
BODY: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
HIDDEN: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
CREATE_PARAMS: TypeAdapter[CreateParams] = TypeAdapter(CreateParams)

ENVIRONMENT_KEYS: Final[frozenset[str]] = frozenset(
    {"OPENAI_API_KEY", "OPENAI_BASE_URL", "OPENAI_API_BASE", "OPENAI_ORGANIZATION"}
)


class ContainerConfig(Protocol):
    def get_supported_openai_params(self) -> list[JsonValue]: ...

    def map_openai_params(
        self,
        params: CreateParams,
        /,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        api_key: str | None = None,
        /,
    ) -> dict[str, str]: ...

    def get_complete_url(
        self,
        api_base: str | None,
        _litellm_params: dict[str, JsonValue],
        /,
    ) -> str: ...

    # ast-grep-ignore: no-long-names
    def transform_container_create_request(
        self,
        name: str,
        params: Mapping[str, JsonValue],
        _litellm_params: GenericLiteLLMParams,
        _headers: dict[str, str],
        /,
    ) -> dict[str, JsonValue]: ...

    # ast-grep-ignore: no-long-names
    def transform_container_create_response(
        self,
        _raw: httpx.Response,
        _logging_obj: Logging,
        /,
    ) -> ContainerObject: ...

    # ast-grep-ignore: no-long-names
    def transform_container_list_request(
        self,
        _api_base: str,
        _litellm_params: GenericLiteLLMParams,
        _headers: dict[str, str],
        /,
        *,
        after: str | None = None,
        limit: int | None = None,
        order: str | None = None,
        extra_query: Mapping[str, JsonValue] | None = None,
    ) -> tuple[str, dict[str, JsonValue]]: ...

    # ast-grep-ignore: no-long-names
    def transform_container_list_response(
        self,
        _raw: httpx.Response,
        _logging_obj: Logging,
        /,
    ) -> ContainerListResponse: ...

    # ast-grep-ignore: no-long-names
    def transform_container_retrieve_request(
        self,
        container_id: str,
        _api_base: str,
        _litellm_params: GenericLiteLLMParams,
        _headers: dict[str, str],
        /,
    ) -> tuple[str, dict[str, JsonValue]]: ...

    # ast-grep-ignore: no-long-names
    def transform_container_retrieve_response(
        self,
        _raw: httpx.Response,
        _logging_obj: Logging,
        /,
    ) -> ContainerObject: ...

    # ast-grep-ignore: no-long-names
    def transform_container_delete_request(
        self,
        container_id: str,
        _api_base: str,
        _litellm_params: GenericLiteLLMParams,
        _headers: dict[str, str],
        /,
    ) -> tuple[str, dict[str, JsonValue]]: ...

    # ast-grep-ignore: no-long-names
    def transform_container_delete_response(
        self,
        _raw: httpx.Response,
        _logging_obj: Logging,
        /,
    ) -> DeleteContainerResult: ...

    # ast-grep-ignore: no-long-names
    def transform_container_file_list_request(
        self,
        _container_id: str,
        _api_base: str,
        _litellm_params: GenericLiteLLMParams,
        _headers: dict[str, str],
        /,
        *,
        after: str | None = None,
        limit: int | None = None,
        order: str | None = None,
        extra_query: Mapping[str, JsonValue] | None = None,
    ) -> tuple[str, dict[str, JsonValue]]: ...

    # ast-grep-ignore: no-long-names
    def transform_container_file_list_response(
        self,
        _raw: httpx.Response,
        _logging_obj: Logging,
        /,
    ) -> ContainerFileListResponse: ...

    # ast-grep-ignore: no-long-names
    def transform_container_file_content_request(
        self,
        _container_id: str,
        _file_id: str,
        _api_base: str,
        _litellm_params: GenericLiteLLMParams,
        _headers: dict[str, str],
        /,
    ) -> tuple[str, dict[str, JsonValue]]: ...

    # ast-grep-ignore: no-long-names
    def transform_container_file_content_response(
        self,
        _raw: httpx.Response,
        _logging_obj: Logging,
        /,
    ) -> bytes: ...

    def get_error_class(
        self,
        error_message: str,
        status_code: int,
        headers: dict[str, str],
        /,
    ) -> BaseLLMException: ...


@contextmanager
def environment(values: Mapping[str, str]) -> Iterator[None]:
    saved: dict[str, str] = {
        name: os.environ[name] for name in ENVIRONMENT_KEYS if name in os.environ
    }
    for name in ENVIRONMENT_KEYS:
        os.environ.pop(name, None)
    os.environ.update(values)
    try:
        yield
    finally:
        for name in ENVIRONMENT_KEYS:
            os.environ.pop(name, None)
        os.environ.update(saved)


@contextmanager
def globals_applied(values: Globals) -> Iterator[None]:
    saved_api_base = litellm.api_base
    saved_api_key = litellm.api_key
    saved_openai_key = litellm.openai_key
    litellm.api_base = values["api_base"]
    litellm.api_key = values["api_key"]
    litellm.openai_key = values["openai_key"]
    try:
        yield
    finally:
        litellm.api_base = saved_api_base
        litellm.api_key = saved_api_key
        litellm.openai_key = saved_openai_key
