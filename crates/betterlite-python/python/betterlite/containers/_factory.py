from collections.abc import Mapping
from typing import Final, Literal, TypeAlias

from pydantic import JsonValue

from betterlite._completion.arguments import TimeoutValues
from betterlite._completion.errors import raise_native_error
from betterlite._endpoints import CallValue, acall_native, call_native
from betterlite._native import NativeContainerFileError, NativeContainerFileSession
from betterlite.containers._crud import DEFAULT_TIMEOUT, call_payload, merged, omit_none
from betterlite.types.containers.main import ContainerFileObject, DeleteContainerFileResponse

RETRIEVE: Final = "retrieve_container_file"
DELETE: Final = "delete_container_file"
CONTENT: Final = "retrieve_container_file_content"
Provider: TypeAlias = Literal["openai", "azure", "azure_text"]
Extra: TypeAlias = dict[str, JsonValue]


def file_values(kwargs: Mapping[str, CallValue], timeout: float | TimeoutValues,
                custom_llm_provider: Provider, extra_headers: Extra | None,
                extra_query: Extra | None, extra_body: Extra | None) -> dict[str, CallValue]:
    named = omit_none(timeout=timeout, custom_llm_provider=custom_llm_provider,
                      extra_headers=extra_headers, extra_query=extra_query,
                      extra_body=extra_body)
    return merged(kwargs, named)


def json_call(operation: str, values: Mapping[str, CallValue]) -> JsonValue:
    session = NativeContainerFileSession("{}")
    return call_native(session.call, call_payload(operation, values), NativeContainerFileError)


async def ajson_call(operation: str, values: Mapping[str, CallValue]) -> JsonValue:
    session = NativeContainerFileSession("{}")
    return await acall_native(session.acall, call_payload(operation, values),
                              NativeContainerFileError)


def bytes_call(operation: str, values: Mapping[str, CallValue]) -> bytes | JsonValue:
    session = NativeContainerFileSession("{}")
    payload = call_payload(operation, values)
    try:
        return session.content(payload.text)
    except NativeContainerFileError as failure:
        raise_native_error(failure)


async def abytes_call(operation: str, values: Mapping[str, CallValue]) -> bytes | JsonValue:
    session = NativeContainerFileSession("{}")
    payload = call_payload(operation, values)
    try:
        return await session.acontent(payload.text)
    except NativeContainerFileError as failure:
        raise_native_error(failure)


def retrieve_container_file(
    timeout: float | TimeoutValues = DEFAULT_TIMEOUT, custom_llm_provider: Provider = "openai",
    extra_headers: Extra | None = None, extra_query: Extra | None = None,
    extra_body: Extra | None = None, **kwargs: CallValue) -> ContainerFileObject:
    return ContainerFileObject.model_validate(json_call(
        RETRIEVE, file_values(kwargs, timeout, custom_llm_provider, extra_headers,
                              extra_query, extra_body)))


async def aretrieve_container_file(
    timeout: float | TimeoutValues = DEFAULT_TIMEOUT, custom_llm_provider: Provider = "openai",
    extra_headers: Extra | None = None, extra_query: Extra | None = None,
    extra_body: Extra | None = None, **kwargs: CallValue) -> ContainerFileObject:
    answer = await ajson_call(RETRIEVE, file_values(
        kwargs, timeout, custom_llm_provider, extra_headers, extra_query, extra_body))
    return ContainerFileObject.model_validate(answer)


def delete_container_file(
    timeout: float | TimeoutValues = DEFAULT_TIMEOUT, custom_llm_provider: Provider = "openai",
    extra_headers: Extra | None = None, extra_query: Extra | None = None,
    extra_body: Extra | None = None, **kwargs: CallValue) -> DeleteContainerFileResponse:
    values = file_values(kwargs, timeout, custom_llm_provider, extra_headers, extra_query,
                         extra_body)
    return DeleteContainerFileResponse.model_validate(json_call(DELETE, values))


async def adelete_container_file(
    timeout: float | TimeoutValues = DEFAULT_TIMEOUT, custom_llm_provider: Provider = "openai",
    extra_headers: Extra | None = None, extra_query: Extra | None = None,
    extra_body: Extra | None = None, **kwargs: CallValue) -> DeleteContainerFileResponse:
    values = merged(kwargs, omit_none(
        timeout=timeout, custom_llm_provider=custom_llm_provider, extra_headers=extra_headers,
        extra_query=extra_query, extra_body=extra_body,
    ))
    return DeleteContainerFileResponse.model_validate(await ajson_call(DELETE, values))


def retrieve_container_file_content(  # ast-grep-ignore: no-long-names
    timeout: float | TimeoutValues = DEFAULT_TIMEOUT, custom_llm_provider: Provider = "openai",
    extra_headers: Extra | None = None, extra_query: Extra | None = None,
    extra_body: Extra | None = None, **kwargs: CallValue) -> bytes | JsonValue:
    values = file_values(kwargs, timeout, custom_llm_provider, extra_headers, extra_query,
                         extra_body)
    return bytes_call(CONTENT, values)


async def aretrieve_container_file_content(  # ast-grep-ignore: no-long-names
    timeout: float | TimeoutValues = DEFAULT_TIMEOUT, custom_llm_provider: Provider = "openai",
    extra_headers: Extra | None = None, extra_query: Extra | None = None,
    extra_body: Extra | None = None, **kwargs: CallValue) -> bytes | JsonValue:
    return await abytes_call(CONTENT, file_values(
        kwargs, timeout, custom_llm_provider, extra_headers, extra_query, extra_body))
