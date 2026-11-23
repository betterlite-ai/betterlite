import builtins
import json
from collections.abc import Awaitable
from typing import Final, Protocol, TypeAlias

from pydantic import JsonValue, TypeAdapter

from betterlite._completion.arguments import JSON, TimeoutValues
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._endpoints import (
    CallPayload,
    CallValue,
    acall_native,
    call_native,
    read_settings,
    to_call_payload,
)
from betterlite._endpoints.payload import REFUSED
from betterlite._native import NativeVectorSearchError, NativeVectorSearchSession
from betterlite.types.vector_stores import (
    VectorStoreCreateResponse,
    VectorStoreFileCounts,
    VectorStoreSearchResponse,
    VectorStoreSearchResult,
)

CREATE_RESPONSE: Final[TypeAdapter[VectorStoreCreateResponse]] = TypeAdapter(
    VectorStoreCreateResponse
)
SEARCH_RESPONSE: Final[TypeAdapter[VectorStoreSearchResponse]] = TypeAdapter(
    VectorStoreSearchResponse
)
SEARCH_RESULTS: Final[TypeAdapter[builtins.list[VectorStoreSearchResult]]] = TypeAdapter(
    builtins.list[VectorStoreSearchResult]
)
SETTINGS: Final[tuple[str, ...]] = ("api_base", "api_key", "openai_key", "organization")
SAMPLE_TEXT: Final = "This is a sample search result from the vector store."
SAMPLE_RESULTS: Final[builtins.list[VectorStoreSearchResult]] = [
    {"score": 0.95, "content": [{"text": SAMPLE_TEXT, "type": "text"}]},
]
Stored: TypeAlias = VectorStoreCreateResponse | VectorStoreSearchResponse


# A Python object a vector store call cannot carry: the router a query is embedded through
class UnsupportedHandle(Protocol):
    ...


# A call answers at once or with a coroutine under its flag, so awaiting either gives the answer
async def settled(pending: Stored | Awaitable[Stored]) -> Stored:
    match pending:
        case Awaitable():
            return await pending
        case object():
            return pending


# ast-grep-ignore: no-long-names
def mock_vector_store_search_response(
    mock_results: builtins.list[VectorStoreSearchResult] | None = None,
) -> VectorStoreSearchResponse:
    data = SAMPLE_RESULTS if mock_results is None else mock_results
    return VectorStoreSearchResponse(
        object="vector_store.search_results.page", search_query="sample query", data=data,
    )


# ast-grep-ignore: no-long-names
def mock_vector_store_create_response(
    mock_response: VectorStoreCreateResponse | None = None,
) -> VectorStoreCreateResponse:
    if mock_response is not None:
        return mock_response
    return VectorStoreCreateResponse(
        id="vs_mock123", object="vector_store", created_at=1699061776,
        name="Mock Vector Store", bytes=0,
        file_counts=VectorStoreFileCounts(
            in_progress=0, completed=0, failed=0, cancelled=0, total=0
        ),
        status="completed", expires_after=None, expires_at=None,
        last_active_at=None, metadata=None,
    )


# a nonempty dict is the create mock, and a null, an empty one or any other value goes to HTTP
def create_mock_answer(given: CallValue) -> VectorStoreCreateResponse | None:
    match given:
        case dict() as fields:
            if fields:
                return mock_vector_store_create_response(
                    CREATE_RESPONSE.validate_python(fields)
                )
            return None
        case object():
            return None


# a nonempty string or list is the search mock, and an empty or other value goes to HTTP
def search_mock_answer(given: CallValue) -> VectorStoreSearchResponse | None:
    match given:
        case str() as text:
            if text:
                return mock_vector_store_search_response()
            return None
        case list() as results:
            if results:
                return mock_vector_store_search_response(
                    SEARCH_RESULTS.validate_python(results)
                )
            return None
        case object():
            return None


# LiteLLM's Python client and session cannot cross into the Rust transport, so they are named back
def refuse_handles(values: dict[str, CallValue]) -> None:
    for name in REFUSED:
        if values.pop(name, None) is not None:
            raise UnsupportedCallArgument(name)


def create_now(arguments: dict[str, CallValue]) -> VectorStoreCreateResponse:
    session = NativeVectorSearchSession(json.dumps(read_settings(SETTINGS)))
    payload: CallPayload = to_call_payload(arguments)
    answer = call_native(session.create, payload, NativeVectorSearchError)
    return CREATE_RESPONSE.validate_python(answer)


async def create_async(arguments: dict[str, CallValue]) -> VectorStoreCreateResponse:
    session = NativeVectorSearchSession(json.dumps(read_settings(SETTINGS)))
    payload: CallPayload = to_call_payload(arguments)
    answer = await acall_native(session.acreate, payload, NativeVectorSearchError)
    return CREATE_RESPONSE.validate_python(answer)


def search_now(arguments: dict[str, CallValue]) -> VectorStoreSearchResponse:
    session = NativeVectorSearchSession(json.dumps(read_settings(SETTINGS)))
    payload: CallPayload = to_call_payload(arguments)
    answer = call_native(session.search, payload, NativeVectorSearchError)
    return SEARCH_RESPONSE.validate_python(answer)


async def search_async(arguments: dict[str, CallValue]) -> VectorStoreSearchResponse:
    session = NativeVectorSearchSession(json.dumps(read_settings(SETTINGS)))
    payload: CallPayload = to_call_payload(arguments)
    answer = await acall_native(session.asearch, payload, NativeVectorSearchError)
    return SEARCH_RESPONSE.validate_python(answer)


def create(
    name: str | None = None,
    file_ids: builtins.list[str] | None = None,
    expires_after: JsonValue = None,
    chunking_strategy: JsonValue = None,
    metadata: dict[str, str] | None = None,
    extra_headers: JsonValue = None,
    extra_query: JsonValue = None,
    extra_body: JsonValue = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreCreateResponse | Awaitable[VectorStoreCreateResponse]:
    mocked = create_mock_answer(kwargs.pop("mock_response", None))
    if mocked is not None:
        return mocked
    refuse_handles(kwargs)
    asynchronous = kwargs.pop("acreate", None) is True
    optional: dict[str, CallValue] = {
        "name": name,
        "file_ids": JSON.validate_python(file_ids),
        "expires_after": expires_after,
        "chunking_strategy": chunking_strategy,
        "metadata": metadata,
    }
    arguments: dict[str, CallValue] = {
        "optional": optional,
        "kwargs": kwargs,
        "extra_headers": extra_headers,
        "extra_body": extra_body,
        "timeout": timeout,
        "custom_llm_provider": custom_llm_provider,
        "entry": "async" if asynchronous else "sync",
    }
    if asynchronous:
        return create_async(arguments)
    return create_now(arguments)


def search(
    vector_store_id: str,
    query: str | builtins.list[str],
    filters: JsonValue = None,
    max_num_results: int | None = None,
    ranking_options: JsonValue = None,
    rewrite_query: bool | None = None,
    extra_headers: JsonValue = None,
    extra_query: JsonValue = None,
    extra_body: JsonValue = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    router: UnsupportedHandle | None = None,
    **kwargs: CallValue,
) -> VectorStoreSearchResponse | Awaitable[VectorStoreSearchResponse]:
    if router is not None:
        raise UnsupportedCallArgument("router")
    executor = kwargs.pop("_direct_vector_store_embedding_executor", None)
    if executor is not None:
        raise UnsupportedCallArgument("_direct_vector_store_embedding_executor")
    mocked = search_mock_answer(kwargs.pop("mock_response", None))
    if mocked is not None:
        return mocked
    refuse_handles(kwargs)
    asynchronous = kwargs.pop("asearch", None) is True
    optional: dict[str, CallValue] = {
        "filters": filters,
        "max_num_results": max_num_results,
        "ranking_options": ranking_options,
        "rewrite_query": rewrite_query,
    }
    arguments: dict[str, CallValue] = {
        "vector_store_id": vector_store_id,
        "query": JSON.validate_python(query),
        "optional": optional,
        "kwargs": kwargs,
        "extra_headers": extra_headers,
        "extra_body": extra_body,
        "timeout": timeout,
        "custom_llm_provider": custom_llm_provider,
        "entry": "async" if asynchronous else "sync",
    }
    if asynchronous:
        return search_async(arguments)
    return search_now(arguments)


async def acreate(
    name: str | None = None, file_ids: builtins.list[str] | None = None,
    expires_after: JsonValue = None, chunking_strategy: JsonValue = None,
    metadata: dict[str, str] | None = None, extra_headers: JsonValue = None,
    extra_query: JsonValue = None, extra_body: JsonValue = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> VectorStoreCreateResponse:
    kwargs.pop("acreate", None)
    pending = create(
        name=name, file_ids=file_ids, expires_after=expires_after,
        chunking_strategy=chunking_strategy, metadata=metadata,
        extra_headers=extra_headers, extra_query=extra_query,
        extra_body=extra_body, timeout=timeout,
        custom_llm_provider=custom_llm_provider, acreate=True, **kwargs,
    )
    return CREATE_RESPONSE.validate_python(await settled(pending))


async def asearch(
    vector_store_id: str, query: str | builtins.list[str],
    filters: JsonValue = None, max_num_results: int | None = None,
    ranking_options: JsonValue = None, rewrite_query: bool | None = None,
    extra_headers: JsonValue = None, extra_query: JsonValue = None,
    extra_body: JsonValue = None, timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None, router: UnsupportedHandle | None = None,
    **kwargs: CallValue,
) -> VectorStoreSearchResponse:
    # LiteLLM's asearch pops the executor at L307-L309; B1059 embeds on its own, so refuse it
    executor = kwargs.pop("_direct_vector_store_embedding_executor", None)
    if executor is not None:
        raise UnsupportedCallArgument("_direct_vector_store_embedding_executor")
    kwargs.pop("asearch", None)
    pending = search(
        vector_store_id=vector_store_id, query=query, filters=filters,
        max_num_results=max_num_results, ranking_options=ranking_options,
        rewrite_query=rewrite_query, extra_headers=extra_headers,
        extra_query=extra_query, extra_body=extra_body, timeout=timeout,
        custom_llm_provider=custom_llm_provider, router=router,
        asearch=True, **kwargs,
    )
    return SEARCH_RESPONSE.validate_python(await settled(pending))
