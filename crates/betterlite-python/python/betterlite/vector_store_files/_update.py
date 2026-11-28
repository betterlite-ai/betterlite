from collections.abc import Awaitable

from pydantic import JsonValue

from betterlite._completion.arguments import TimeoutValues
from betterlite._endpoints import CallValue
from betterlite.types.vector_store_files import VectorStoreFileObject
from betterlite.vector_store_files._calls import (
    FILE,
    call_values,
    dispatched,
    update_async,
    update_now,
)


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
    values = call_values(
        kwargs, vector_store_id=vector_store_id, file_id=file_id,
        attributes=attributes, extra_headers=extra_headers, extra_body=extra_body,
        timeout=timeout, custom_llm_provider=custom_llm_provider,
    )
    return dispatched(values, kwargs, "aupdate", update_async, update_now)


async def aupdate(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L524
    *,
    vector_store_id: str,
    file_id: str,
    attributes: JsonValue,
    extra_headers: JsonValue = None,
    extra_body: JsonValue = None,
    timeout: float | TimeoutValues | None = None,  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L531
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreFileObject:
    kwargs.pop("aupdate", None)
    values = call_values(
        kwargs, vector_store_id=vector_store_id, file_id=file_id,
        attributes=attributes, extra_headers=extra_headers, extra_body=extra_body,
        timeout=timeout, custom_llm_provider=custom_llm_provider,
    )
    return FILE.validate_python(await update_async(values))
