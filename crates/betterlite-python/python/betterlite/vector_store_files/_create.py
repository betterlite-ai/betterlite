from collections.abc import Awaitable

from pydantic import JsonValue

from betterlite._completion.arguments import TimeoutValues
from betterlite._endpoints import CallValue
from betterlite.types.vector_store_files import VectorStoreFileObject
from betterlite.vector_store_files._calls import (
    FILE,
    call_values,
    create_async,
    create_now,
    dispatched,
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
        extra_headers=extra_headers, extra_query=extra_query, extra_body=extra_body,
        timeout=timeout, custom_llm_provider=custom_llm_provider,
    )
    return dispatched(values, kwargs, "acreate", create_async, create_now)


async def acreate(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L55
    *,
    vector_store_id: str,
    file_id: str,
    attributes: JsonValue = None,
    chunking_strategy: JsonValue = None,
    extra_headers: JsonValue = None,
    extra_query: JsonValue = None,
    extra_body: JsonValue = None,
    timeout: float | TimeoutValues | None = None,  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L64
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreFileObject:
    kwargs.pop("acreate", None)
    values = call_values(
        kwargs, vector_store_id=vector_store_id, file_id=file_id,
        attributes=attributes, chunking_strategy=chunking_strategy,
        extra_headers=extra_headers, extra_query=extra_query, extra_body=extra_body,
        timeout=timeout, custom_llm_provider=custom_llm_provider,
    )
    return FILE.validate_python(await create_async(values))
