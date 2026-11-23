import asyncio
import importlib
import inspect
from collections.abc import Awaitable
from typing import Final, Protocol

import pytest
from pydantic import JsonValue, TypeAdapter
from vector_server import (
    BAD_KEY,
    GOOD_KEY,
    SEARCH_PATH,
    STORE_ID,
    STORE_NAME,
    STORE_PATH,
    VectorServer,
    header_of,
)

from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._native import NativeVectorSearchError
from betterlite.exceptions import AuthenticationError
from betterlite.types.vector_stores import (
    VectorStoreCreateResponse,
    VectorStoreSearchResponse,
    VectorStoreSearchResult,
)
from betterlite.vector_stores._create_search import (
    acreate,
    asearch,
    create,
    mock_vector_store_create_response,
    mock_vector_store_search_response,
    search,
)

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
QUERY_TEXT: Final = "the vector store"
QUERY_PARTS: Final[list[str]] = ["alpha", "beta"]
FILE_IDS: Final[list[str]] = ["file-1", "file-2"]
METADATA: Final[dict[str, str]] = {"team": "search"}
FILTERS: Final[dict[str, JsonValue]] = {"type": "eq", "key": "source", "value": "doc.md"}
RANKING: Final[dict[str, JsonValue]] = {"ranker": "default-2024-11-15"}
RESULTS: Final[list[VectorStoreSearchResult]] = [
    {
        "score": 0.5,
        "content": [{"text": "hit", "type": "text"}],
        "file_id": "file-1",
        "filename": "doc.md",
        "attributes": {"source": "doc.md"},
    }
]
SEEDED: Final[VectorStoreCreateResponse] = {"id": "vs_seeded", "object": "vector_store"}


class VectorCall(Protocol):
    def __call__(self, **kwargs: JsonValue) -> dict[str, JsonValue]: ...


class VectorAsyncCall(Protocol):
    def __call__(self, **kwargs: JsonValue) -> Awaitable[dict[str, JsonValue]]: ...


class VectorStores(Protocol):
    def __getattr__(self, name: str) -> VectorCall: ...


class VectorStoresAsync(Protocol):
    def __getattr__(self, name: str) -> VectorAsyncCall: ...


class HelperCall(Protocol):
    def __call__(self, **kwargs: JsonValue) -> dict[str, JsonValue]: ...


class VectorStoreHelpers(Protocol):
    def __getattr__(self, name: str) -> HelperCall: ...


class ExceptionClass(Protocol):
    def __getattr__(self, name: str) -> type[BaseException]: ...


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


class HandleStub:
    def read(self) -> bytes:
        return b""


LITELLM: VectorStores = importlib.import_module("litellm.vector_stores")
LITELLM_ASYNC: VectorStoresAsync = importlib.import_module("litellm.vector_stores")
LITELLM_HELPERS: VectorStoreHelpers = importlib.import_module("litellm.vector_stores.main")
LITELLM_ERRORS: ExceptionClass = importlib.import_module("litellm")


def parameter_rows(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        default: DefaultView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(default.default)))
    return rows


async def settled_create(
    pending: VectorStoreCreateResponse | Awaitable[VectorStoreCreateResponse],
) -> VectorStoreCreateResponse:
    match pending:
        case Awaitable():
            return await pending
        case object():
            return pending


async def settled_search(
    pending: VectorStoreSearchResponse | Awaitable[VectorStoreSearchResponse],
) -> VectorStoreSearchResponse:
    match pending:
        case Awaitable():
            return await pending
        case object():
            return pending


async def settled_flags(
    flagged_create: VectorStoreCreateResponse | Awaitable[VectorStoreCreateResponse],
    flagged_search: VectorStoreSearchResponse | Awaitable[VectorStoreSearchResponse],
) -> tuple[VectorStoreCreateResponse, VectorStoreSearchResponse]:
    created = await settled_create(flagged_create)
    searched = await settled_search(flagged_search)
    return (created, searched)


async def resolved_calls(
    ours_create: Awaitable[VectorStoreCreateResponse],
    ours_search: Awaitable[VectorStoreSearchResponse],
    reference_create: Awaitable[dict[str, JsonValue]],
    reference_search: Awaitable[dict[str, JsonValue]],
) -> tuple[
    VectorStoreCreateResponse,
    VectorStoreSearchResponse,
    dict[str, JsonValue],
    dict[str, JsonValue],
]:
    created = await ours_create
    searched = await ours_search
    reference_created = await reference_create
    reference_searched = await reference_search
    return (created, searched, reference_created, reference_searched)


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(create)) == parameter_rows(
        inspect.signature(LITELLM.create)
    )
    assert parameter_rows(inspect.signature(search)) == parameter_rows(
        inspect.signature(LITELLM.search)
    )
    assert parameter_rows(inspect.signature(acreate)) == parameter_rows(
        inspect.signature(LITELLM.acreate)
    )
    assert parameter_rows(inspect.signature(asearch)) == parameter_rows(
        inspect.signature(LITELLM.asearch)
    )
    assert inspect.iscoroutinefunction(acreate)
    assert inspect.iscoroutinefunction(asearch)
    assert not inspect.iscoroutinefunction(create)
    assert not inspect.iscoroutinefunction(search)


def test_create_calls_match_litellm() -> None:
    with VectorServer() as server:
        ours = create(
            name=STORE_NAME,
            file_ids=FILE_IDS,
            metadata=METADATA,
            extra_headers={"X-Custom": "yes"},
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.calls[-1]
        reference = LITELLM.create(
            name=STORE_NAME,
            file_ids=FILE_IDS,
            metadata=METADATA,
            extra_headers={"X-Custom": "yes"},
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.calls[-1]
    assert ours_call["path"] == STORE_PATH
    assert ours_call["path"] == reference_call["path"]
    assert ours_call["body"] == reference_call["body"]
    assert ours_call["body"]["name"] == STORE_NAME
    assert ours_call["body"]["file_ids"] == FILE_IDS
    assert ours_call["body"]["metadata"] == METADATA
    assert header_of(ours_call, "Authorization") == header_of(reference_call, "Authorization")
    assert header_of(ours_call, "Authorization") == f"Bearer {GOOD_KEY}"
    assert header_of(ours_call, "Content-Type") == header_of(reference_call, "Content-Type")
    assert header_of(ours_call, "OpenAI-Beta") == header_of(reference_call, "OpenAI-Beta")
    assert header_of(ours_call, "OpenAI-Beta") == "assistants=v2"
    assert header_of(ours_call, "X-Custom") == "yes"
    assert JSON.validate_python(ours) == JSON.validate_python(reference)


def test_search_calls_match_litellm() -> None:
    with VectorServer() as server:
        ours = search(
            vector_store_id=STORE_ID,
            query=QUERY_PARTS,
            filters=FILTERS,
            max_num_results=2,
            ranking_options=RANKING,
            rewrite_query=True,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.calls[-1]
        reference = LITELLM.search(
            vector_store_id=STORE_ID,
            query=QUERY_PARTS,
            filters=FILTERS,
            max_num_results=2,
            ranking_options=RANKING,
            rewrite_query=True,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.calls[-1]
    assert ours_call["path"] == SEARCH_PATH
    assert ours_call["path"] == reference_call["path"]
    assert ours_call["body"] == reference_call["body"]
    assert ours_call["body"]["query"] == QUERY_PARTS
    assert ours_call["body"]["filters"] == FILTERS
    assert ours_call["body"]["max_num_results"] == 2
    assert ours_call["body"]["ranking_options"] == RANKING
    assert ours_call["body"]["rewrite_query"] is True
    assert header_of(ours_call, "Authorization") == f"Bearer {GOOD_KEY}"
    assert JSON.validate_python(ours) == JSON.validate_python(reference)


def test_search_defaults_calls_match_litellm() -> None:
    with VectorServer() as server:
        ours = search(
            vector_store_id=STORE_ID,
            query=QUERY_TEXT,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.calls[-1]
        reference = LITELLM.search(
            vector_store_id=STORE_ID,
            query=QUERY_TEXT,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.calls[-1]
    assert ours_call["body"] == reference_call["body"]
    assert ours_call["body"]["query"] == QUERY_TEXT
    assert ours_call["body"]["filters"] is None
    assert ours_call["body"]["max_num_results"] is None
    assert ours_call["body"]["ranking_options"] is None
    assert ours_call["body"]["rewrite_query"] is None
    assert JSON.validate_python(ours) == JSON.validate_python(reference)


def test_async_calls_match_litellm() -> None:
    with VectorServer() as server:
        results = asyncio.run(
            resolved_calls(
                acreate(name=STORE_NAME, api_base=server.url, api_key=GOOD_KEY),
                asearch(
                    vector_store_id=STORE_ID,
                    query=QUERY_TEXT,
                    api_base=server.url,
                    api_key=GOOD_KEY,
                ),
                LITELLM_ASYNC.acreate(
                    name=STORE_NAME, api_base=server.url, api_key=GOOD_KEY
                ),
                LITELLM_ASYNC.asearch(
                    vector_store_id=STORE_ID,
                    query=QUERY_TEXT,
                    api_base=server.url,
                    api_key=GOOD_KEY,
                ),
            )
        )
    created, searched, reference_created, reference_searched = results
    assert JSON.validate_python(created) == JSON.validate_python(reference_created)
    assert JSON.validate_python(searched) == JSON.validate_python(reference_searched)


def test_flagged_calls_are_coroutines() -> None:
    with VectorServer() as server:
        plain_create = create(name=STORE_NAME, api_base=server.url, api_key=GOOD_KEY)
        plain_search = search(
            vector_store_id=STORE_ID, query=QUERY_TEXT, api_base=server.url, api_key=GOOD_KEY
        )
        flagged_create = create(
            name=STORE_NAME, api_base=server.url, api_key=GOOD_KEY, acreate=True
        )
        flagged_search = search(
            vector_store_id=STORE_ID,
            query=QUERY_TEXT,
            api_base=server.url,
            api_key=GOOD_KEY,
            asearch=True,
        )
        assert inspect.isawaitable(flagged_create)
        assert inspect.isawaitable(flagged_search)
        created, searched = asyncio.run(settled_flags(flagged_create, flagged_search))
    assert JSON.validate_python(created) == JSON.validate_python(plain_create)
    assert JSON.validate_python(searched) == JSON.validate_python(plain_search)


def test_mock_helpers_match_litellm() -> None:
    ours_search = mock_vector_store_search_response()
    reference_search = LITELLM_HELPERS.mock_vector_store_search_response()
    assert JSON.validate_python(ours_search) == JSON.validate_python(reference_search)
    ours_create = mock_vector_store_create_response()
    reference_create = LITELLM_HELPERS.mock_vector_store_create_response()
    assert JSON.validate_python(ours_create) == JSON.validate_python(reference_create)


def test_mock_results_match_litellm() -> None:
    ours = mock_vector_store_search_response(RESULTS)
    reference = LITELLM_HELPERS.mock_vector_store_search_response(
        mock_results=JSON.validate_python(RESULTS)
    )
    assert JSON.validate_python(ours) == JSON.validate_python(reference)


def test_create_mock_response_matches_litellm() -> None:
    reference = LITELLM_HELPERS.mock_vector_store_create_response(
        mock_response=JSON.validate_python(SEEDED)
    )
    with VectorServer() as server:
        ours = create(
            name=STORE_NAME,
            mock_response=JSON.validate_python(SEEDED),
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        assert server.calls == []
    assert JSON.validate_python(ours) == JSON.validate_python(reference)


def test_search_mock_string_matches_litellm() -> None:
    with VectorServer() as server:
        ours = search(
            vector_store_id=STORE_ID,
            query=QUERY_TEXT,
            mock_response="hello",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference = LITELLM.search(
            vector_store_id=STORE_ID,
            query=QUERY_TEXT,
            mock_response="hello",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        assert server.calls == []
    assert JSON.validate_python(ours) == JSON.validate_python(reference)


def test_search_mock_list_matches_litellm() -> None:
    with VectorServer() as server:
        ours = search(
            vector_store_id=STORE_ID,
            query=QUERY_TEXT,
            mock_response=JSON.validate_python(RESULTS),
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference = LITELLM.search(
            vector_store_id=STORE_ID,
            query=QUERY_TEXT,
            mock_response=JSON.validate_python(RESULTS),
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        assert server.calls == []
    assert JSON.validate_python(ours) == JSON.validate_python(reference)


def test_refused_handles_raise() -> None:
    with pytest.raises(UnsupportedCallArgument, match="router"):
        search(vector_store_id=STORE_ID, query=QUERY_TEXT, router=HandleStub())
    with pytest.raises(UnsupportedCallArgument, match="_direct_vector_store_embedding_executor"):
        search(
            vector_store_id=STORE_ID,
            query=QUERY_TEXT,
            _direct_vector_store_embedding_executor=HandleStub(),
        )
    with pytest.raises(UnsupportedCallArgument, match="client"):
        create(name=STORE_NAME, client=HandleStub())
    with pytest.raises(UnsupportedCallArgument, match="client"):
        search(vector_store_id=STORE_ID, query=QUERY_TEXT, client=HandleStub())
    with pytest.raises(UnsupportedCallArgument, match="shared_session"):
        create(name=STORE_NAME, shared_session=HandleStub())


def test_provider_error_keeps_class_and_cause() -> None:
    reference_class = LITELLM_ERRORS.AuthenticationError
    with VectorServer() as server:
        with pytest.raises(AuthenticationError) as ours_caught:
            search(
                vector_store_id=STORE_ID,
                query=QUERY_TEXT,
                api_base=server.url,
                api_key=BAD_KEY,
            )
        with pytest.raises(reference_class):
            LITELLM.search(
                vector_store_id=STORE_ID,
                query=QUERY_TEXT,
                api_base=server.url,
                api_key=BAD_KEY,
            )
    assert type(ours_caught.value).__name__ == reference_class.__name__
    assert ours_caught.value.status_code == 401
    match ours_caught.value.__cause__:
        case NativeVectorSearchError():
            pass
        case object():
            raise AssertionError("the provider error carries no native cause")
