from collections.abc import Awaitable

from pydantic import JsonValue

from betterlite._completion.arguments import TimeoutValues
from betterlite._endpoints import CallValue
from betterlite.types.vector_store_files import VectorStoreFileContentResponse
from betterlite.vector_store_files._calls import (
    CONTENT,
    call_values,
    content_async,
    content_now,
    dispatched,
)


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
    return dispatched(values, kwargs, "aretrieve_content", content_async, content_now)


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
    values = call_values(
        kwargs, vector_store_id=vector_store_id, file_id=file_id,
        extra_headers=extra_headers, timeout=timeout,
        custom_llm_provider=custom_llm_provider,
    )
    return CONTENT.validate_python(await content_async(values))
