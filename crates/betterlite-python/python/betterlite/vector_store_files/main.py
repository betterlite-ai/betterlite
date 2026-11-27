from collections.abc import Awaitable

from pydantic import JsonValue

from betterlite._completion.arguments import TimeoutValues
from betterlite._endpoints import CallValue
from betterlite.types.vector_store_files import (
    VectorStoreFileContentResponse,
    VectorStoreFileDeleteResponse,
    VectorStoreFileListResponse,
    VectorStoreFileObject,
)
from betterlite.vector_store_files._calls import (
    CONTENT,
    DELETED,
    FILE,
    PAGE,
    call_values,
    connection_values,
    content_async,
    content_now,
    create_async,
    create_now,
    delete_async,
    delete_now,
    list_async,
    list_now,
    retrieve_async,
    retrieve_now,
    settled,
    update_async,
    update_now,
)


def create(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L107
    *,
    vector_store_id: str,
    file_id: str,
    attributes: JsonValue = None,
    chunking_strategy: JsonValue = None,
    extra_headers: JsonValue = None,
    extra_query: JsonValue = None,
    extra_body: JsonValue = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreFileObject | Awaitable[VectorStoreFileObject]:
    values = call_values(
        kwargs, vector_store_id=vector_store_id, file_id=file_id,
        attributes=attributes, chunking_strategy=chunking_strategy,
        **connection_values(
            extra_headers=extra_headers, extra_query=extra_query, extra_body=extra_body,
            timeout=timeout, custom_llm_provider=custom_llm_provider,
        ),
    )
    match kwargs.pop("acreate", None):
        case given if given is True:
            return create_async(values)
        case _:
            return create_now(values)


async def acreate(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L55
    *,
    vector_store_id: str,
    file_id: str,
    attributes: JsonValue = None,
    chunking_strategy: JsonValue = None,
    extra_headers: JsonValue = None,
    extra_query: JsonValue = None,
    extra_body: JsonValue = None,
    timeout: float | TimeoutValues | None = None,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L64
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreFileObject:
    kwargs.pop("acreate", None)
    pending = create(
        vector_store_id=vector_store_id, file_id=file_id, attributes=attributes,
        chunking_strategy=chunking_strategy, extra_headers=extra_headers,
        extra_query=extra_query, extra_body=extra_body, timeout=timeout,
        custom_llm_provider=custom_llm_provider, acreate=True, **kwargs,
    )
    return FILE.validate_python(await settled(pending))


def list(  # noqa: A001, PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L235
    *,
    vector_store_id: str,
    after: str | None = None,
    before: str | None = None,
    filter: str | None = None,  # noqa: A002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L240
    limit: int | None = None,
    order: str | None = None,
    extra_headers: JsonValue = None,
    extra_query: JsonValue = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreFileListResponse | Awaitable[VectorStoreFileListResponse]:
    values = call_values(
        kwargs, vector_store_id=vector_store_id, extra_headers=extra_headers,
        extra_query=extra_query, timeout=timeout, custom_llm_provider=custom_llm_provider,
    )
    query = connection_values(
        after=after, before=before, filter=filter, limit=limit, order=order,
    )
    values.update(query)
    if kwargs.pop("alist", None) is True:
        return list_async(values)
    return list_now(values)


async def alist(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L182
    *,
    vector_store_id: str,
    after: str | None = None,
    before: str | None = None,
    filter: str | None = None,  # noqa: A002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L187
    limit: int | None = None,
    order: str | None = None,
    extra_headers: JsonValue = None,
    extra_query: JsonValue = None,
    timeout: float | TimeoutValues | None = None,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L192
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreFileListResponse:
    kwargs.pop("alist", None)
    values = call_values(
        kwargs, vector_store_id=vector_store_id, extra_headers=extra_headers,
        extra_query=extra_query, timeout=timeout, custom_llm_provider=custom_llm_provider,
    )
    query = connection_values(
        after=after, before=before, filter=filter, limit=limit, order=order,
    )
    values |= query
    return PAGE.validate_python(await settled(list_async(values)))


def retrieve(
    *,
    vector_store_id: str,
    file_id: str,
    extra_headers: JsonValue = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreFileObject | Awaitable[VectorStoreFileObject]:
    values = call_values(
        kwargs, vector_store_id=vector_store_id, file_id=file_id,
        extra_headers=extra_headers, timeout=timeout,
        custom_llm_provider=custom_llm_provider,
    )
    if kwargs.pop("aretrieve", None) is True:
        return retrieve_async(values)
    return retrieve_now(values)


async def aretrieve(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L307
    *,
    vector_store_id: str,
    file_id: str,
    extra_headers: JsonValue = None,
    timeout: float | TimeoutValues | None = None,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L312
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreFileObject:
    kwargs.pop("aretrieve", None)
    answer = await retrieve_async(call_values(
        kwargs, vector_store_id=vector_store_id, file_id=file_id,
        extra_headers=extra_headers, timeout=timeout,
        custom_llm_provider=custom_llm_provider,
    ))
    return FILE.validate_python(answer)


def retrieve_content(
    *,
    vector_store_id: str,
    file_id: str,
    extra_headers: JsonValue = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreFileContentResponse | Awaitable[VectorStoreFileContentResponse]:
    values = call_values(
        kwargs, vector_store_id=vector_store_id, file_id=file_id,
        extra_headers=extra_headers, timeout=timeout,
        custom_llm_provider=custom_llm_provider,
    )
    content_call = content_async if kwargs.pop("aretrieve_content", None) is True else content_now
    return content_call(values)


async def aretrieve_content(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L416
    *,
    vector_store_id: str,
    file_id: str,
    extra_headers: JsonValue = None,
    timeout: float | TimeoutValues | None = None,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L421
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreFileContentResponse:
    kwargs.pop("aretrieve_content", None)
    return CONTENT.validate_python(await settled(content_async(call_values(
        kwargs, vector_store_id=vector_store_id, file_id=file_id,
        extra_headers=extra_headers, timeout=timeout,
        custom_llm_provider=custom_llm_provider,
    ))))


def update(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L570
    *,
    vector_store_id: str,
    file_id: str,
    attributes: JsonValue,
    extra_headers: JsonValue = None,
    extra_body: JsonValue = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreFileObject | Awaitable[VectorStoreFileObject]:
    values = call_values(kwargs, vector_store_id=vector_store_id, file_id=file_id)
    values["attributes"] = attributes
    values |= connection_values(
        extra_headers=extra_headers, extra_body=extra_body, timeout=timeout,
        custom_llm_provider=custom_llm_provider,
    )
    return (
        update_now(values)
        if kwargs.pop("aupdate", None) is not True
        else update_async(values)
    )


async def aupdate(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L524
    *,
    vector_store_id: str,
    file_id: str,
    attributes: JsonValue,
    extra_headers: JsonValue = None,
    extra_body: JsonValue = None,
    timeout: float | TimeoutValues | None = None,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L531
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreFileObject:
    kwargs.pop("aupdate", None)
    values = call_values(kwargs, vector_store_id=vector_store_id, file_id=file_id)
    values["attributes"] = attributes
    values |= connection_values(
        extra_headers=extra_headers, extra_body=extra_body, timeout=timeout,
        custom_llm_provider=custom_llm_provider,
    )
    answer = await update_async(values)
    return FILE.validate_python(answer)


def delete(
    *,
    vector_store_id: str,
    file_id: str,
    extra_headers: JsonValue = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreFileDeleteResponse | Awaitable[VectorStoreFileDeleteResponse]:
    values = call_values(kwargs, vector_store_id=vector_store_id, file_id=file_id)
    values |= connection_values(
        extra_headers=extra_headers, timeout=timeout,
        custom_llm_provider=custom_llm_provider,
    )
    if kwargs.pop("adelete", None) is True:
        return delete_async(values)
    return delete_now(values)


async def adelete(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L645
    *,
    vector_store_id: str,
    file_id: str,
    extra_headers: JsonValue = None,
    timeout: float | TimeoutValues | None = None,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L650
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreFileDeleteResponse:
    kwargs.pop("adelete", None)
    values = call_values(kwargs, vector_store_id=vector_store_id, file_id=file_id)
    values |= connection_values(
        extra_headers=extra_headers, timeout=timeout,
        custom_llm_provider=custom_llm_provider,
    )
    return DELETED.validate_python(await delete_async(values))
