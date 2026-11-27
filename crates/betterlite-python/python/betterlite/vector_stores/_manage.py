import json
from collections.abc import Awaitable
from typing import Final, TypeAlias

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
from betterlite._native import NativeVectorManageError, NativeVectorManageSession
from betterlite.types.vector_stores import VectorStoreCreateResponse
from betterlite.vector_stores._create_search import SETTINGS, drop_bookkeeping, refuse_handles

CREATE_RESPONSE: Final[TypeAdapter[VectorStoreCreateResponse]] = TypeAdapter(
    VectorStoreCreateResponse
)
Managed: TypeAlias = VectorStoreCreateResponse | dict[str, JsonValue]


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


# retrieve and delete take the same request: the id, the call's **kwargs and its transport fields
def managed_arguments(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L547
    vector_store_id: str,
    kwargs: dict[str, CallValue],
    extra_headers: dict[str, JsonValue] | None,
    extra_query: dict[str, JsonValue] | None,
    extra_body: dict[str, JsonValue] | None,
    timeout: float | TimeoutValues | None,
    custom_llm_provider: str | None,
) -> dict[str, CallValue]:
    arguments: dict[str, CallValue] = {
        "vector_store_id": vector_store_id,
        "kwargs": kwargs,
        "extra_headers": extra_headers,
        "extra_query": extra_query,
        "extra_body": extra_body,
        "timeout": timeout,
        "custom_llm_provider": custom_llm_provider,
    }
    return arguments


def retrieve_now(arguments: dict[str, CallValue]) -> VectorStoreCreateResponse:
    payload: CallPayload = to_call_payload(arguments)
    answer = call_native(manage_session().retrieve, payload, NativeVectorManageError)
    return CREATE_RESPONSE.validate_python(answer)


async def retrieve_async(arguments: dict[str, CallValue]) -> VectorStoreCreateResponse:
    payload: CallPayload = to_call_payload(arguments)
    answer = await acall_native(manage_session().aretrieve, payload, NativeVectorManageError)
    return CREATE_RESPONSE.validate_python(answer)


def list_now(arguments: dict[str, CallValue]) -> dict[str, JsonValue]:
    payload: CallPayload = to_call_payload(arguments)
    answer = call_native(manage_session().list, payload, NativeVectorManageError)
    return JSON_OBJECT.validate_python(answer)


async def list_async(arguments: dict[str, CallValue]) -> dict[str, JsonValue]:
    payload: CallPayload = to_call_payload(arguments)
    answer = await acall_native(manage_session().alist, payload, NativeVectorManageError)
    return JSON_OBJECT.validate_python(answer)


def update_now(arguments: dict[str, CallValue]) -> VectorStoreCreateResponse:
    payload: CallPayload = to_call_payload(arguments)
    answer = call_native(manage_session().update, payload, NativeVectorManageError)
    return CREATE_RESPONSE.validate_python(answer)


async def update_async(arguments: dict[str, CallValue]) -> VectorStoreCreateResponse:
    payload: CallPayload = to_call_payload(arguments)
    answer = await acall_native(manage_session().aupdate, payload, NativeVectorManageError)
    return CREATE_RESPONSE.validate_python(answer)


def delete_now(arguments: dict[str, CallValue]) -> dict[str, JsonValue]:
    payload: CallPayload = to_call_payload(arguments)
    answer = call_native(manage_session().delete, payload, NativeVectorManageError)
    return JSON_OBJECT.validate_python(answer)


async def delete_async(arguments: dict[str, CallValue]) -> dict[str, JsonValue]:
    payload: CallPayload = to_call_payload(arguments)
    answer = await acall_native(manage_session().adelete, payload, NativeVectorManageError)
    return JSON_OBJECT.validate_python(answer)


def retrieve(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L547
    vector_store_id: str,
    extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreCreateResponse | Awaitable[VectorStoreCreateResponse]:
    refuse_handles(kwargs)
    drop_bookkeeping(kwargs)
    asynchronous = kwargs.pop("aretrieve", None) is True
    arguments = managed_arguments(
        vector_store_id, kwargs, extra_headers, extra_query,
        extra_body, timeout, custom_llm_provider,
    )
    if asynchronous:
        return retrieve_async(arguments)
    return retrieve_now(arguments)


def list(  # noqa: A001, PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L685
    after: str | None = None,
    before: str | None = None,
    limit: int | None = 20,
    order: str | None = "desc",
    extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> dict[str, JsonValue] | Awaitable[dict[str, JsonValue]]:
    refuse_handles(kwargs)
    drop_bookkeeping(kwargs)
    asynchronous = kwargs.pop("alist", None) is True
    arguments: dict[str, CallValue] = {
        "after": after,
        "before": before,
        "limit": limit,
        "order": order,
        "kwargs": kwargs,
    }
    arguments |= {
        "extra_headers": extra_headers,
        "extra_query": extra_query,
        "extra_body": extra_body,
        "timeout": timeout,
        "custom_llm_provider": custom_llm_provider,
    }
    if asynchronous:
        return list_async(arguments)
    return list_now(arguments)


def update(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L837
    vector_store_id: str,
    name: str | None = None,
    expires_after: JsonValue = None,
    metadata: dict[str, str] | None = None,
    extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreCreateResponse | Awaitable[VectorStoreCreateResponse]:
    refuse_handles(kwargs)
    drop_bookkeeping(kwargs)
    asynchronous = kwargs.pop("aupdate", None) is True
    # only fields the caller set are sent; an explicit empty metadata is still a field to send
    optional: dict[str, CallValue] = {
        "name": name,
        "expires_after": expires_after,
        "metadata": metadata,
    }
    arguments: dict[str, CallValue] = {
        "vector_store_id": vector_store_id,
        "kwargs": kwargs,
        **optional,
        "extra_headers": extra_headers,
        "extra_query": extra_query,
        "extra_body": extra_body,
        "timeout": timeout,
        "custom_llm_provider": custom_llm_provider,
    }
    return update_async(arguments) if asynchronous else update_now(arguments)


def delete(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L986
    vector_store_id: str,
    extra_headers: dict[str, JsonValue] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> dict[str, JsonValue] | Awaitable[dict[str, JsonValue]]:
    refuse_handles(kwargs)
    drop_bookkeeping(kwargs)
    asynchronous = kwargs.pop("adelete", None) is True
    arguments = managed_arguments(
        vector_store_id, kwargs, extra_headers, extra_query,
        extra_body, timeout, custom_llm_provider,
    )
    return delete_async(arguments) if asynchronous else delete_now(arguments)


async def aretrieve(vector_store_id: str, extra_headers: dict[str, JsonValue] | None = None, extra_query: dict[str, JsonValue] | None = None, extra_body: dict[str, JsonValue] | None = None, timeout: float | TimeoutValues | None = None, custom_llm_provider: str | None = None, **kwargs: CallValue) -> VectorStoreCreateResponse:  # noqa: ASYNC109, PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L495
    kwargs.pop("aretrieve", None)
    pending = retrieve(
        vector_store_id=vector_store_id,
        extra_headers=extra_headers,
        extra_query=extra_query,
        extra_body=extra_body,
        timeout=timeout,
        custom_llm_provider=custom_llm_provider,
        aretrieve=True,
        **kwargs,
    )
    return CREATE_RESPONSE.validate_python(await settled(pending))


async def alist(after: str | None = None, before: str | None = None, limit: int | None = 20, order: str | None = "desc", extra_headers: dict[str, JsonValue] | None = None, extra_query: dict[str, JsonValue] | None = None, extra_body: dict[str, JsonValue] | None = None, timeout: float | TimeoutValues | None = None, custom_llm_provider: str | None = None, **kwargs: CallValue) -> dict[str, JsonValue]:  # noqa: ASYNC109, PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L627
    kwargs.pop("alist", None)
    pending = list(
        after, before, limit, order, extra_headers, extra_query, extra_body,
        timeout, custom_llm_provider, alist=True, **kwargs,
    )
    return JSON_OBJECT.validate_python(await settled(pending))


async def aupdate(vector_store_id: str, name: str | None = None, expires_after: JsonValue = None, metadata: dict[str, str] | None = None, extra_headers: dict[str, JsonValue] | None = None, extra_query: dict[str, JsonValue] | None = None, extra_body: dict[str, JsonValue] | None = None, timeout: float | TimeoutValues | None = None, custom_llm_provider: str | None = None, **kwargs: CallValue) -> VectorStoreCreateResponse:  # noqa: ASYNC109, PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L779
    kwargs.pop("aupdate", None)
    pending = update(
        vector_store_id=vector_store_id,
        name=name,
        expires_after=expires_after,
        metadata=metadata,
        extra_headers=extra_headers,
        extra_query=extra_query,
        extra_body=extra_body,
        timeout=timeout,
        custom_llm_provider=custom_llm_provider,
        aupdate=True,
        **kwargs,
    )
    answer = await settled(pending)
    return CREATE_RESPONSE.validate_python(answer)


async def adelete(vector_store_id: str, extra_headers: dict[str, JsonValue] | None = None, extra_query: dict[str, JsonValue] | None = None, extra_body: dict[str, JsonValue] | None = None, timeout: float | TimeoutValues | None = None, custom_llm_provider: str | None = None, **kwargs: CallValue) -> dict[str, JsonValue]:  # noqa: ASYNC109, PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L934
    kwargs["adelete"] = True
    pending = delete(
        vector_store_id, extra_headers, extra_query, extra_body, timeout,
        custom_llm_provider, **kwargs,
    )
    answer = await settled(pending)
    return JSON_OBJECT.validate_python(answer)
