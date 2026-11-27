from collections.abc import Awaitable

from pydantic import JsonValue

from betterlite._completion.arguments import TimeoutValues
from betterlite._endpoints import CallValue
from betterlite.types.vector_store_files import VectorStoreFileListResponse
from betterlite.vector_store_files._calls import (
    PAGE,
    call_values,
    dispatched,
    list_async,
    list_now,
)
from betterlite.vector_store_files._content import aretrieve_content, retrieve_content
from betterlite.vector_store_files._create import acreate, create
from betterlite.vector_store_files._delete import adelete, delete
from betterlite.vector_store_files._retrieve import aretrieve, retrieve
from betterlite.vector_store_files._update import aupdate, update

__all__ = [
    "acreate",
    "adelete",
    "alist",
    "aretrieve",
    "aretrieve_content",
    "aupdate",
    "create",
    "delete",
    "list",
    "retrieve",
    "retrieve_content",
    "update",
]


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
        kwargs, vector_store_id=vector_store_id, after=after, before=before,
        filter=filter, limit=limit, order=order, extra_headers=extra_headers,
        extra_query=extra_query, timeout=timeout, custom_llm_provider=custom_llm_provider,
    )
    return dispatched(values, kwargs, "alist", list_async, list_now)


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
        kwargs, vector_store_id=vector_store_id, after=after, before=before,
        filter=filter, limit=limit, order=order, extra_headers=extra_headers,
        extra_query=extra_query, timeout=timeout, custom_llm_provider=custom_llm_provider,
    )
    return PAGE.validate_python(await list_async(values))
