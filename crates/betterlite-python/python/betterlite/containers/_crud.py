from collections.abc import Mapping
from typing import Final, Literal, TypeAlias

from pydantic import JsonValue, TypeAdapter

from betterlite._completion.arguments import TimeoutValues
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._endpoints import (
    CallPayload,
    CallValue,
    acall_native,
    call_native,
    read_settings,
    to_call_payload,
)
from betterlite._native import NativeContainerError, NativeContainerSession
from betterlite.types.containers.main import (
    ContainerListResponse,
    ContainerObject,
    DeleteContainerResult,
)

DEFAULT_TIMEOUT: Final = 600
# the module globals a container call falls back to, read at call time like LiteLLM's own
SETTINGS: Final[tuple[str, ...]] = ("api_base", "api_key", "openai_key", "organization")
# LiteLLM pops these two before it reads the call, so they never become arguments
BOOKKEEPING: Final[tuple[str, ...]] = ("async_call", "litellm_logging_obj")
# a token provider is a Python callable the payload cannot carry into Rust
REFUSED: Final = "azure_ad_token_provider"

OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
Provider: TypeAlias = Literal["openai", "azure", "azure_text"]


# the call as the session takes it: the arguments, the module settings and what Rust drops
def call_payload(values: Mapping[str, CallValue]) -> CallPayload:
    if values.get(REFUSED) is not None:
        raise UnsupportedCallArgument(REFUSED)
    arguments = {name: given for name, given in values.items() if given is not None}
    kept = {name: given for name, given in arguments.items() if name not in BOOKKEEPING}
    return to_call_payload({**kept, "settings": read_settings(SETTINGS)})


# a keyword the caller left unset stays out, so the fallback it would hide applies instead
def merged(
    kwargs: Mapping[str, CallValue], named: Mapping[str, CallValue]
) -> dict[str, CallValue]:
    together = {**kwargs, **named}
    return {name: value for name, value in together.items() if value is not None}


# only the values the caller set, for the operations that name each one at the call
def omit_none(**named: CallValue) -> dict[str, CallValue]:
    given = named.items()
    return {name: value for name, value in given if value is not None}


# retrieve and delete take the same request, and a managed id inside container_id decodes in Rust
def operation_values(
    kwargs: Mapping[str, CallValue],
    container_id: str,
    timeout: float | TimeoutValues,
    api_key: str | None,
    api_base: str | None,
    api_version: str | None,
    custom_llm_provider: Provider,
    extra_headers: dict[str, JsonValue] | None,
    extra_query: dict[str, JsonValue] | None,
    extra_body: dict[str, JsonValue] | None,
) -> dict[str, CallValue]:
    named = omit_none(
        container_id=container_id, timeout=timeout, api_key=api_key, api_base=api_base,
        api_version=api_version, custom_llm_provider=custom_llm_provider,
        extra_headers=extra_headers, extra_query=extra_query, extra_body=extra_body,
    )
    return merged(kwargs, named)


# the created container as LiteLLM returns it: the record plus the hidden params its transform set
def created(answer: JsonValue) -> ContainerObject:
    fields = OBJECT.validate_python(answer)
    hidden = OBJECT.validate_python(fields.pop("_hidden_params", {}))
    container = ContainerObject.model_validate(fields)
    # a pydantic private attribute lives in this dict, which is where LiteLLM's assignment lands
    private = container.__pydantic_private__
    if private is None:
        raise RuntimeError("ContainerObject keeps no private attribute storage")
    private["_hidden_params"] = hidden
    return container


def create_from_values(values: Mapping[str, CallValue]) -> ContainerObject:
    answer = call_native(
        NativeContainerSession("{}").create_container, call_payload(values), NativeContainerError
    )
    return created(answer)


async def acreate_from_values(values: Mapping[str, CallValue]) -> ContainerObject:
    answer = await acall_native(
        NativeContainerSession("{}").acreate_container, call_payload(values),
        NativeContainerError,
    )
    return created(answer)


def list_from_values(values: Mapping[str, CallValue]) -> ContainerListResponse:
    answer = call_native(
        NativeContainerSession("{}").list_containers, call_payload(values), NativeContainerError
    )
    return ContainerListResponse.model_validate(answer)


async def alist_from_values(values: Mapping[str, CallValue]) -> ContainerListResponse:
    answer = await acall_native(
        NativeContainerSession("{}").alist_containers, call_payload(values), NativeContainerError
    )
    return ContainerListResponse.model_validate(answer)


async def aretrieve_from_values(values: Mapping[str, CallValue]) -> ContainerObject:
    answer = await acall_native(
        NativeContainerSession("{}").aretrieve_container, call_payload(values),
        NativeContainerError,
    )
    return ContainerObject.model_validate(answer)


def create_container(
    name: str,
    expires_after: dict[str, JsonValue] | None = None,
    file_ids: list[str] | None = None,
    timeout: float | TimeoutValues = DEFAULT_TIMEOUT,
    api_key: str | None = None,
    api_base: str | None = None,
    api_version: str | None = None,
    custom_llm_provider: Provider = "openai",
    extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> ContainerObject:
    named: dict[str, CallValue] = {
        "name": name, "expires_after": expires_after, "file_ids": JSON.validate_python(file_ids),
        "timeout": timeout, "api_key": api_key, "api_base": api_base, "api_version": api_version,
        "custom_llm_provider": custom_llm_provider, "extra_headers": extra_headers,
        "extra_query": extra_query, "extra_body": extra_body,
    }
    return create_from_values(merged(kwargs, named))


async def acreate_container(
    name: str,
    expires_after: dict[str, JsonValue] | None = None,
    file_ids: list[str] | None = None,
    timeout: float | TimeoutValues = DEFAULT_TIMEOUT,
    custom_llm_provider: Provider = "openai",
    extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> ContainerObject:
    return await acreate_from_values(merged(kwargs, {
        "name": name, "expires_after": expires_after, "file_ids": JSON.validate_python(file_ids),
        "timeout": timeout, "custom_llm_provider": custom_llm_provider,
        "extra_headers": extra_headers, "extra_query": extra_query, "extra_body": extra_body,
    }))


def list_containers(
    after: str | None = None,
    limit: int | None = None,
    order: str | None = None,
    timeout: float | TimeoutValues = DEFAULT_TIMEOUT,
    api_key: str | None = None,
    api_base: str | None = None,
    api_version: str | None = None,
    custom_llm_provider: Provider = "openai",
    extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> ContainerListResponse:
    values: dict[str, CallValue] = {
        "timeout": timeout, "api_key": api_key, "api_base": api_base,
        "api_version": api_version, "custom_llm_provider": custom_llm_provider,
        "extra_headers": extra_headers, "extra_query": extra_query, "extra_body": extra_body,
    }
    # the query carries after, limit and order only when the caller set them
    for name, value in (("after", after), ("limit", limit), ("order", order)):
        if value is not None:
            values[name] = value
    return list_from_values(merged(kwargs, values))


async def alist_containers(
    after: str | None = None,
    limit: int | None = None,
    order: str | None = None,
    timeout: float | TimeoutValues = DEFAULT_TIMEOUT,
    custom_llm_provider: Provider = "openai",
    extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> ContainerListResponse:
    values = {
        **kwargs,
        **omit_none(
            after=after, limit=limit, order=order, timeout=timeout,
            custom_llm_provider=custom_llm_provider, extra_headers=extra_headers,
            extra_query=extra_query, extra_body=extra_body,
        ),
    }
    return await alist_from_values(values)


def retrieve_container(
    container_id: str,
    timeout: float | TimeoutValues = DEFAULT_TIMEOUT,
    api_key: str | None = None,
    api_base: str | None = None,
    api_version: str | None = None,
    custom_llm_provider: Provider = "openai",
    extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> ContainerObject:
    values = operation_values(
        kwargs, container_id, timeout, api_key, api_base, api_version,
        custom_llm_provider, extra_headers, extra_query, extra_body,
    )
    answer = call_native(
        NativeContainerSession("{}").retrieve_container, call_payload(values), NativeContainerError
    )
    return ContainerObject.model_validate(answer)


async def aretrieve_container(
    container_id: str,
    timeout: float | TimeoutValues = DEFAULT_TIMEOUT,
    custom_llm_provider: Provider = "openai",
    extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> ContainerObject:
    return await aretrieve_from_values(
        operation_values(
            kwargs, container_id, timeout, None, None, None,
            custom_llm_provider, extra_headers, extra_query, extra_body,
        )
    )


def delete_container(
    container_id: str,
    timeout: float | TimeoutValues = DEFAULT_TIMEOUT,
    api_key: str | None = None,
    api_base: str | None = None,
    api_version: str | None = None,
    custom_llm_provider: Provider = "openai",
    extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> DeleteContainerResult:
    session = NativeContainerSession("{}")
    values = operation_values(
        kwargs, container_id, timeout, api_key, api_base, api_version,
        custom_llm_provider, extra_headers, extra_query, extra_body,
    )
    return DeleteContainerResult.model_validate(
        call_native(session.delete_container, call_payload(values), NativeContainerError)
    )


async def adelete_container(
    container_id: str,
    timeout: float | TimeoutValues = DEFAULT_TIMEOUT,
    custom_llm_provider: Provider = "openai",
    extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> DeleteContainerResult:
    values = {
        **kwargs,
        **omit_none(
            container_id=container_id, timeout=timeout,
            custom_llm_provider=custom_llm_provider, extra_headers=extra_headers,
            extra_query=extra_query, extra_body=extra_body,
        ),
    }
    answer = await acall_native(
        NativeContainerSession("{}").adelete_container, call_payload(values), NativeContainerError
    )
    return DeleteContainerResult.model_validate(answer)
