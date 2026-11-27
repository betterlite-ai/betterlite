from collections.abc import Awaitable

from pydantic import JsonValue

from betterlite._completion.arguments import TimeoutValues
from betterlite._endpoints import CallValue
from betterlite.types.vector_store_files import VectorStoreFileDeleteResponse
from betterlite.vector_store_files._calls import (
    DELETED,
    call_values,
    delete_async,
    delete_now,
    dispatched,
)


def delete(
    *,
    vector_store_id: str,
    file_id: str,
    extra_headers: JsonValue = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreFileDeleteResponse | Awaitable[VectorStoreFileDeleteResponse]:
    values = call_values(
        kwargs, vector_store_id=vector_store_id, file_id=file_id,
        extra_headers=extra_headers, timeout=timeout,
        custom_llm_provider=custom_llm_provider,
    )
    return dispatched(values, kwargs, "adelete", delete_async, delete_now)


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
    values = call_values(
        kwargs, vector_store_id=vector_store_id, file_id=file_id,
        extra_headers=extra_headers, timeout=timeout,
        custom_llm_provider=custom_llm_provider,
    )
    return DELETED.validate_python(await delete_async(values))
