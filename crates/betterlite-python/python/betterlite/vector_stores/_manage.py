import json
from collections.abc import Awaitable
from typing import Final, TypeAlias, TypeVar

from pydantic import JsonValue, TypeAdapter

from betterlite._completion.arguments import JSON_OBJECT, TimeoutValues
from betterlite._endpoints import (
    CallPayload,
    CallValue,
    acall_native,
    call_native,
    read_settings,
    to_call_payload,
)
from betterlite._endpoints.runner import AsyncNativeCall, NativeCall
from betterlite._native import NativeVectorManageError, NativeVectorManageSession
from betterlite.types.vector_stores import VectorStoreCreateResponse
from betterlite.vector_stores._create_search import SETTINGS, drop_bookkeeping, refuse_handles

CREATE_RESPONSE: Final[TypeAdapter[VectorStoreCreateResponse]] = TypeAdapter(
    VectorStoreCreateResponse
)
Managed: TypeAlias = VectorStoreCreateResponse | dict[str, JsonValue]
Answer = TypeVar("Answer")


# a flagged sync call answers with a coroutine, so awaiting either form ends the call
async def settled(pending: Managed | Awaitable[Managed]) -> Managed:
    match pending:
        case Awaitable():
            return await pending
        case object():
            return pending


def manage_session() -> NativeVectorManageSession:
    settings = json.dumps(read_settings(SETTINGS))
    return NativeVectorManageSession(settings)


# the transport fields every one of the four calls carries, in the shape the native session reads
def transport_args(
    extra_headers: dict[str, JsonValue] | None,
    extra_query: dict[str, JsonValue] | None,
    extra_body: dict[str, JsonValue] | None,
    timeout: float | TimeoutValues | None,
    custom_llm_provider: str | None,
) -> dict[str, CallValue]:
    return {
        "extra_headers": extra_headers,
        "extra_query": extra_query,
        "extra_body": extra_body,
        "timeout": timeout,
        "custom_llm_provider": custom_llm_provider,
    }


def query_fields(
    after: str | None, before: str | None, limit: int | None, order: str | None
) -> dict[str, CallValue]:
    return {
        "after": after, "before": before, "limit": limit, "order": order,
    }


def update_fields(
    store_id: str, name: str | None, expires_after: JsonValue, metadata: dict[str, str] | None
) -> dict[str, CallValue]:
    return {
        "vector_store_id": store_id, "name": name,
        "expires_after": expires_after, "metadata": metadata,
    }


def managed_arguments(
    fields: dict[str, CallValue], kwargs: dict[str, CallValue], transport: dict[str, CallValue]
) -> dict[str, CallValue]:
    return {
        **fields, "transport": {**transport, "kwargs": kwargs},
    }


def call_kwargs(values: dict[str, CallValue], flag: str) -> tuple[dict[str, CallValue], bool]:
    refuse_handles(values)
    drop_bookkeeping(values)
    return values, values.pop(flag, None) is True


def now(
    method: NativeCall, arguments: dict[str, CallValue], adapter: TypeAdapter[Answer]
) -> Answer:
    payload: CallPayload = to_call_payload(arguments)
    return adapter.validate_python(call_native(method, payload, NativeVectorManageError))


async def later(
    method: AsyncNativeCall, arguments: dict[str, CallValue], adapter: TypeAdapter[Answer]
) -> Answer:
    payload: CallPayload = to_call_payload(arguments)
    returned = await acall_native(method, payload, NativeVectorManageError)
    return adapter.validate_python(returned)


def retrieve(vector_store_id: str, extra_headers: dict[str, JsonValue] | None = None, extra_query: dict[str, JsonValue] | None = None, extra_body: dict[str, JsonValue] | None = None, timeout: float | TimeoutValues | None = None, custom_llm_provider: str | None = None, **kwargs: CallValue) -> VectorStoreCreateResponse | Awaitable[VectorStoreCreateResponse]:  # noqa: E501, PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L547
    kwargs, asynchronous = call_kwargs(kwargs, "aretrieve")
    transport = transport_args(extra_headers, extra_query, extra_body, timeout, custom_llm_provider)
    arguments = managed_arguments({"vector_store_id": vector_store_id}, kwargs, transport)
    if asynchronous:
        return later(manage_session().aretrieve, arguments, CREATE_RESPONSE)
    return now(manage_session().retrieve, arguments, CREATE_RESPONSE)


def list(after: str | None = None, before: str | None = None, limit: int | None = 20, order: str | None = "desc", extra_headers: dict[str, JsonValue] | None = None, extra_query: dict[str, JsonValue] | None = None, extra_body: dict[str, JsonValue] | None = None, timeout: float | TimeoutValues | None = None, custom_llm_provider: str | None = None, **kwargs: CallValue) -> dict[str, JsonValue] | Awaitable[dict[str, JsonValue]]:  # noqa: A001, E501, PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L685
    kwargs, asynchronous = call_kwargs(kwargs, "alist")
    transport = transport_args(extra_headers, extra_query, extra_body, timeout, custom_llm_provider)
    arguments = managed_arguments(query_fields(after, before, limit, order), kwargs, transport)
    if asynchronous:
        return later(manage_session().alist, arguments, JSON_OBJECT)
    return now(manage_session().list, arguments, JSON_OBJECT)


def update(vector_store_id: str, name: str | None = None, expires_after: JsonValue = None, metadata: dict[str, str] | None = None, extra_headers: dict[str, JsonValue] | None = None, extra_query: dict[str, JsonValue] | None = None, extra_body: dict[str, JsonValue] | None = None, timeout: float | TimeoutValues | None = None, custom_llm_provider: str | None = None, **kwargs: CallValue) -> VectorStoreCreateResponse | Awaitable[VectorStoreCreateResponse]:  # noqa: E501, PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L837
    kwargs, asynchronous = call_kwargs(kwargs, "aupdate")
    transport = transport_args(extra_headers, extra_query, extra_body, timeout, custom_llm_provider)
    fields = update_fields(vector_store_id, name, expires_after, metadata)
    arguments = managed_arguments(fields, kwargs, transport)
    if asynchronous:
        return later(manage_session().aupdate, arguments, CREATE_RESPONSE)
    return now(manage_session().update, arguments, CREATE_RESPONSE)


def delete(vector_store_id: str, extra_headers: dict[str, JsonValue] | None = None, extra_query: dict[str, JsonValue] | None = None, extra_body: dict[str, JsonValue] | None = None, timeout: float | TimeoutValues | None = None, custom_llm_provider: str | None = None, **kwargs: CallValue) -> dict[str, JsonValue] | Awaitable[dict[str, JsonValue]]:  # noqa: E501, PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L986
    kwargs, asynchronous = call_kwargs(kwargs, "adelete")
    transport = transport_args(extra_headers, extra_query, extra_body, timeout, custom_llm_provider)
    arguments = managed_arguments({"vector_store_id": vector_store_id}, kwargs, transport)
    if asynchronous:
        return later(manage_session().adelete, arguments, JSON_OBJECT)
    return now(manage_session().delete, arguments, JSON_OBJECT)


async def aretrieve(vector_store_id: str, extra_headers: dict[str, JsonValue] | None = None, extra_query: dict[str, JsonValue] | None = None, extra_body: dict[str, JsonValue] | None = None, timeout: float | TimeoutValues | None = None, custom_llm_provider: str | None = None, **kwargs: CallValue) -> VectorStoreCreateResponse:  # noqa: ASYNC109, E501, PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L495
    kwargs.pop("aretrieve", None)
    pending = retrieve(
        vector_store_id, extra_headers, extra_query, extra_body, timeout,
        custom_llm_provider, aretrieve=True, **kwargs,
    )
    return CREATE_RESPONSE.validate_python(await settled(pending))


async def alist(after: str | None = None, before: str | None = None, limit: int | None = 20, order: str | None = "desc", extra_headers: dict[str, JsonValue] | None = None, extra_query: dict[str, JsonValue] | None = None, extra_body: dict[str, JsonValue] | None = None, timeout: float | TimeoutValues | None = None, custom_llm_provider: str | None = None, **kwargs: CallValue) -> dict[str, JsonValue]:  # noqa: ASYNC109, E501, PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L627
    kwargs.pop("alist", None)
    pending = list(
        after, before, limit, order, extra_headers, extra_query, extra_body,
        timeout, custom_llm_provider, alist=True, **kwargs,
    )
    return JSON_OBJECT.validate_python(await settled(pending))


async def aupdate(vector_store_id: str, name: str | None = None, expires_after: JsonValue = None, metadata: dict[str, str] | None = None, extra_headers: dict[str, JsonValue] | None = None, extra_query: dict[str, JsonValue] | None = None, extra_body: dict[str, JsonValue] | None = None, timeout: float | TimeoutValues | None = None, custom_llm_provider: str | None = None, **kwargs: CallValue) -> VectorStoreCreateResponse:  # noqa: ASYNC109, E501, PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L779
    kwargs.pop("aupdate", None)
    pending = update(
        vector_store_id, name, expires_after, metadata, extra_headers, extra_query,
        extra_body, timeout, custom_llm_provider, aupdate=True, **kwargs,
    )
    return CREATE_RESPONSE.validate_python(await settled(pending))


async def adelete(vector_store_id: str, extra_headers: dict[str, JsonValue] | None = None, extra_query: dict[str, JsonValue] | None = None, extra_body: dict[str, JsonValue] | None = None, timeout: float | TimeoutValues | None = None, custom_llm_provider: str | None = None, **kwargs: CallValue) -> dict[str, JsonValue]:  # noqa: ASYNC109, E501, PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L934
    kwargs.pop("adelete", None)
    pending = delete(
        vector_store_id, extra_headers, extra_query, extra_body, timeout,
        custom_llm_provider, adelete=True, **kwargs,
    )
    return JSON_OBJECT.validate_python(await settled(pending))
