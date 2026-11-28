from collections.abc import Awaitable

from pydantic import JsonValue

from betterlite._completion.arguments import TimeoutValues
from betterlite._endpoints import CallValue
from betterlite.types.vector_store_files import VectorStoreFileObject
from betterlite.vector_store_files._calls import (
    FILE,
    call_values,
    dispatched,
    retrieve_async,
    retrieve_now,
)


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
    return dispatched(values, kwargs, "aretrieve", retrieve_async, retrieve_now)


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
    values = call_values(
        kwargs, vector_store_id=vector_store_id, file_id=file_id,
        extra_headers=extra_headers, timeout=timeout,
        custom_llm_provider=custom_llm_provider,
    )
    return FILE.validate_python(await retrieve_async(values))
