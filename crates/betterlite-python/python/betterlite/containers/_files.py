from typing import Final, Literal, TypeAlias

from pydantic import JsonValue

from betterlite._completion.arguments import TimeoutValues
from betterlite._endpoints import CallValue, acall_native, call_native
from betterlite._native import NativeContainerFileError, NativeContainerFileSession
from betterlite.containers._crud import DEFAULT_TIMEOUT, call_payload, merged, omit_none
from betterlite.types.containers.main import ContainerFileListResponse, ContainerFileObject
from betterlite.types.llms.openai.core.aliases import FileTypes

LIST: Final = "list_container_files"
UPLOAD: Final = "upload_container_file"
Provider: TypeAlias = Literal["openai", "azure", "azure_text"]


def list_container_files(
    container_id: str, after: str | None = None, limit: int | None = None,
    order: str | None = None, timeout: float | TimeoutValues = DEFAULT_TIMEOUT,
    api_key: str | None = None, api_base: str | None = None, api_version: str | None = None,
    custom_llm_provider: Provider = "openai", extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None, **kwargs: CallValue,
) -> ContainerFileListResponse:
    values = merged(kwargs, omit_none(
        container_id=container_id, after=after, limit=limit, order=order, timeout=timeout,
        api_key=api_key, api_base=api_base, api_version=api_version,
        custom_llm_provider=custom_llm_provider, extra_headers=extra_headers,
        extra_query=extra_query, extra_body=extra_body,
    ))
    answer = call_native(
        NativeContainerFileSession("{}").call, call_payload(LIST, values),
        NativeContainerFileError,
    )
    return ContainerFileListResponse.model_validate(answer)


async def alist_container_files(
    container_id: str, after: str | None = None, limit: int | None = None,
    order: str | None = None, timeout: float | TimeoutValues = DEFAULT_TIMEOUT,
    custom_llm_provider: Provider = "openai", extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None, **kwargs: CallValue,
) -> ContainerFileListResponse:
    return ContainerFileListResponse.model_validate(await acall_native(
        NativeContainerFileSession("{}").acall,
        call_payload(LIST, merged(kwargs, omit_none(
            container_id=container_id, after=after, limit=limit, order=order, timeout=timeout,
            custom_llm_provider=custom_llm_provider, extra_headers=extra_headers,
            extra_query=extra_query, extra_body=extra_body,
        ))),
        NativeContainerFileError,
    ))


def upload_container_file(
    container_id: str, file: FileTypes, timeout: float | TimeoutValues = DEFAULT_TIMEOUT,
    api_key: str | None = None, api_base: str | None = None, api_version: str | None = None,
    custom_llm_provider: Provider = "openai", extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None, **kwargs: CallValue,
) -> ContainerFileObject:
    named: dict[str, CallValue] = {
        "container_id": container_id, "file": file, "timeout": timeout, "api_key": api_key,
        "api_base": api_base, "api_version": api_version,
        "custom_llm_provider": custom_llm_provider, "extra_headers": extra_headers,
        "extra_query": extra_query, "extra_body": extra_body,
    }
    return ContainerFileObject.model_validate(call_native(
        NativeContainerFileSession("{}").call, call_payload(UPLOAD, merged(kwargs, named)),
        NativeContainerFileError,
    ))


async def aupload_container_file(
    container_id: str, file: FileTypes, timeout: float | TimeoutValues = DEFAULT_TIMEOUT,
    custom_llm_provider: Provider = "openai", extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None, **kwargs: CallValue,
) -> ContainerFileObject:
    values = merged(kwargs, {
        "container_id": container_id, "file": file, "timeout": timeout,
        "custom_llm_provider": custom_llm_provider, "extra_headers": extra_headers,
        "extra_query": extra_query, "extra_body": extra_body,
    })
    return ContainerFileObject.model_validate(
        await acall_native(NativeContainerFileSession("{}").acall,
                           call_payload(UPLOAD, values), NativeContainerFileError)
    )
