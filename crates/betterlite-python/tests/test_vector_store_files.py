import asyncio
import importlib
import inspect
import logging
from collections.abc import Awaitable
from types import ModuleType
from typing import Final, Protocol, TypeVar

import pytest
from pydantic import JsonValue, TypeAdapter
from vector_files_server import (
    ATTRIBUTES,
    BAD_KEY,
    CONTENT_PATH,
    FILE_ID,
    FILE_PATH,
    GOOD_KEY,
    STORE_ID,
    STORE_PATH,
    VectorFilesServer,
    header_of,
)

import betterlite
from betterlite import vector_store_files as vector_file_package
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._endpoints.payload import LOGGER_FN_DROPPED
from betterlite._native import NativeVectorFileError
from betterlite.exceptions import APIConnectionError, AuthenticationError
from betterlite.vector_store_files._calls import (
    CONTENT,
    DELETED,
    FILE,
    PAGE,
    settled,
)
from betterlite.vector_store_files.main import (
    acreate,
    adelete,
    alist,
    aretrieve,
    aretrieve_content,
    aupdate,
    create,
    delete,
    retrieve,
    retrieve_content,
    update,
)
from betterlite.vector_store_files.main import (
    list as list_files,
)

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ResultT = TypeVar("ResultT")
EXTRA_HEADERS: Final[JsonValue] = {"X-Custom": "yes"}
EXTRA_QUERY: Final[JsonValue] = {"q": "1"}
EXTRA_BODY: Final[JsonValue] = {"custom_field": "extra"}
CHUNKING_STRATEGY: Final[JsonValue] = {"type": "auto"}
UPDATED_ATTRIBUTES: Final[JsonValue] = {"source": "new.md"}
CUSTOM_PROVIDER: Final = "anthropic"
CONTENT_TYPE: Final = "application/json"
ASSISTANTS_HEADER: Final = "assistants=v2"


class FileCall(Protocol):
    def __call__(self, **kwargs: JsonValue) -> dict[str, JsonValue]: ...


class AsyncFileCall(Protocol):
    def __call__(self, **kwargs: JsonValue) -> Awaitable[dict[str, JsonValue]]: ...


class VectorFileCalls(Protocol):
    def __getattr__(self, name: str) -> FileCall: ...


class AsyncVectorFileCalls(Protocol):
    def __getattr__(self, name: str) -> AsyncFileCall: ...


class ExceptionClass(Protocol):
    def __getattr__(self, name: str) -> type[BaseException]: ...


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


LITELLM: VectorFileCalls = importlib.import_module("litellm.vector_store_files")
LITELLM_ASYNC: AsyncVectorFileCalls = importlib.import_module("litellm.vector_store_files")
LITELLM_ERRORS: ExceptionClass = importlib.import_module("litellm")
LITELLM_MODULE: Final[ModuleType] = importlib.import_module("litellm")


def parameter_rows(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        default: DefaultView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(default.default)))
    return rows


def compare(ours: ResultT, reference: dict[str, JsonValue]) -> None:
    expected = JSON.validate_python(reference)
    assert JSON.validate_python(ours) == expected


async def awaited_pair(
    ours: Awaitable[ResultT],
    reference: Awaitable[dict[str, JsonValue]],
) -> tuple[ResultT, dict[str, JsonValue]]:
    ours_result = await ours
    reference_result = await reference
    return (ours_result, reference_result)


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(create)) == parameter_rows(
        inspect.signature(LITELLM.create)
    )
    assert parameter_rows(inspect.signature(acreate)) == parameter_rows(
        inspect.signature(LITELLM.acreate)
    )
    assert parameter_rows(inspect.signature(list_files)) == parameter_rows(
        inspect.signature(LITELLM.list)
    )
    assert parameter_rows(inspect.signature(alist)) == parameter_rows(
        inspect.signature(LITELLM.alist)
    )
    assert parameter_rows(inspect.signature(retrieve)) == parameter_rows(
        inspect.signature(LITELLM.retrieve)
    )
    assert parameter_rows(inspect.signature(aretrieve)) == parameter_rows(
        inspect.signature(LITELLM.aretrieve)
    )
    assert parameter_rows(inspect.signature(retrieve_content)) == parameter_rows(
        inspect.signature(LITELLM.retrieve_content)
    )
    assert parameter_rows(inspect.signature(aretrieve_content)) == parameter_rows(
        inspect.signature(LITELLM.aretrieve_content)
    )
    assert parameter_rows(inspect.signature(update)) == parameter_rows(
        inspect.signature(LITELLM.update)
    )
    assert parameter_rows(inspect.signature(aupdate)) == parameter_rows(
        inspect.signature(LITELLM.aupdate)
    )
    assert parameter_rows(inspect.signature(delete)) == parameter_rows(
        inspect.signature(LITELLM.delete)
    )
    assert parameter_rows(inspect.signature(adelete)) == parameter_rows(
        inspect.signature(LITELLM.adelete)
    )


def test_signature_exports() -> None:
    exported: Final[tuple[str, ...]] = (
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
    )
    assert tuple(vector_file_package.__all__) == exported
    assert vector_file_package.acreate is acreate
    assert vector_file_package.adelete is adelete
    assert vector_file_package.alist is alist
    assert vector_file_package.aretrieve is aretrieve
    assert vector_file_package.aretrieve_content is aretrieve_content
    assert vector_file_package.aupdate is aupdate
    assert vector_file_package.create is create
    assert vector_file_package.delete is delete
    assert vector_file_package.list is list_files
    assert vector_file_package.retrieve is retrieve
    assert vector_file_package.retrieve_content is retrieve_content
    assert vector_file_package.update is update


def test_calls_create() -> None:
    with VectorFilesServer() as server:
        ours = create(
            vector_store_id=STORE_ID,
            file_id=FILE_ID,
            attributes=ATTRIBUTES,
            chunking_strategy=CHUNKING_STRATEGY,
            extra_headers=EXTRA_HEADERS,
            extra_query=EXTRA_QUERY,
            extra_body=EXTRA_BODY,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference = LITELLM.create(
            vector_store_id=STORE_ID,
            file_id=FILE_ID,
            attributes=JSON.validate_python(ATTRIBUTES),
            chunking_strategy=CHUNKING_STRATEGY,
            extra_headers=EXTRA_HEADERS,
            extra_query=EXTRA_QUERY,
            extra_body=EXTRA_BODY,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call, reference_call = server.calls
    assert ours_call["method"] == reference_call["method"] == "POST"
    assert ours_call["route"] == reference_call["route"] == STORE_PATH
    assert ours_call["body"] == reference_call["body"]
    assert ours_call["body"]["file_id"] == FILE_ID
    assert ours_call["body"]["attributes"] == ATTRIBUTES
    assert ours_call["body"]["chunking_strategy"] == CHUNKING_STRATEGY
    # the create handler merges extra_body, and its handler never reads extra_query
    assert ours_call["body"]["custom_field"] == "extra"
    assert ours_call["query"] == reference_call["query"] == {}
    headers: dict[str, str] = {
        "Authorization": f"Bearer {GOOD_KEY}",
        "Content-Type": CONTENT_TYPE,
        "OpenAI-Beta": ASSISTANTS_HEADER,
        "X-Custom": "yes",
    }
    for header, value in headers.items():
        assert header_of(ours_call, header) == header_of(reference_call, header) == value
    compare(ours, reference)


def test_calls_list() -> None:
    with VectorFilesServer() as server:
        ours = list_files(
            vector_store_id=STORE_ID,
            after=FILE_ID,
            before=FILE_ID,
            filter="completed",
            limit=3,
            order="desc",
            extra_headers=EXTRA_HEADERS,
            extra_query=EXTRA_QUERY,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference = LITELLM.list(
            vector_store_id=STORE_ID,
            after=FILE_ID,
            before=FILE_ID,
            filter="completed",
            limit=3,
            order="desc",
            extra_headers=EXTRA_HEADERS,
            extra_query=EXTRA_QUERY,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call, reference_call = server.calls
    expected: dict[str, str] = {
        "after": FILE_ID,
        "before": FILE_ID,
        "filter": "completed",
        "limit": "3",
        "order": "desc",
        "q": "1",
    }
    assert ours_call["query"] == reference_call["query"] == expected
    assert ours_call["method"] == reference_call["method"] == "GET"
    assert ours_call["route"] == reference_call["route"] == STORE_PATH
    assert header_of(ours_call, "Authorization") == f"Bearer {GOOD_KEY}"
    assert header_of(ours_call, "OpenAI-Beta") == ASSISTANTS_HEADER
    assert header_of(ours_call, "X-Custom") == header_of(reference_call, "X-Custom") == "yes"
    compare(ours, reference)


def test_calls_retrieve() -> None:
    with VectorFilesServer() as server:
        ours = retrieve(
            vector_store_id=STORE_ID,
            file_id=FILE_ID,
            extra_headers=EXTRA_HEADERS,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference = LITELLM.retrieve(
            vector_store_id=STORE_ID,
            file_id=FILE_ID,
            extra_headers=EXTRA_HEADERS,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call, reference_call = server.calls
    assert ours_call["method"] == reference_call["method"] == "GET"
    assert ours_call["route"] == reference_call["route"] == FILE_PATH
    assert ours_call["query"] == reference_call["query"] == {}
    expected_headers: dict[str, str] = {
        "Authorization": f"Bearer {GOOD_KEY}",
        "Content-Type": CONTENT_TYPE,
        "OpenAI-Beta": ASSISTANTS_HEADER,
        "X-Custom": "yes",
    }
    for header, value in expected_headers.items():
        assert header_of(ours_call, header) == header_of(reference_call, header) == value
    compare(ours, reference)


def test_calls_retrieve_content() -> None:
    with VectorFilesServer() as server:
        ours = retrieve_content(
            vector_store_id=STORE_ID,
            file_id=FILE_ID,
            extra_headers=EXTRA_HEADERS,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference = LITELLM.retrieve_content(
            vector_store_id=STORE_ID,
            file_id=FILE_ID,
            extra_headers=EXTRA_HEADERS,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call, reference_call = server.calls
    assert ours_call["method"] == reference_call["method"] == "GET"
    assert ours_call["route"] == reference_call["route"] == CONTENT_PATH
    assert ours_call["route"].endswith("/content")
    assert ours_call["query"] == reference_call["query"] == {}
    assert header_of(ours_call, "Authorization") == header_of(reference_call, "Authorization")
    assert header_of(ours_call, "Authorization") == f"Bearer {GOOD_KEY}"
    compare(ours, reference)


def test_calls_update() -> None:
    with VectorFilesServer() as server:
        ours = update(
            vector_store_id=STORE_ID,
            file_id=FILE_ID,
            attributes=UPDATED_ATTRIBUTES,
            extra_headers=EXTRA_HEADERS,
            extra_body=EXTRA_BODY,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference = LITELLM.update(
            vector_store_id=STORE_ID,
            file_id=FILE_ID,
            attributes=JSON.validate_python(UPDATED_ATTRIBUTES),
            extra_headers=EXTRA_HEADERS,
            extra_body=EXTRA_BODY,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call, reference_call = server.calls
    expected: dict[str, JsonValue] = {
        "attributes": {"source": "new.md"},
        "custom_field": "extra",
    }
    assert ours_call["method"] == reference_call["method"] == "POST"
    assert ours_call["route"] == reference_call["route"] == FILE_PATH
    assert ours_call["body"] == reference_call["body"] == expected
    assert ours_call["query"] == reference_call["query"] == {}
    headers: dict[str, str] = {
        "Authorization": f"Bearer {GOOD_KEY}",
        "Content-Type": CONTENT_TYPE,
        "OpenAI-Beta": ASSISTANTS_HEADER,
        "X-Custom": "yes",
    }
    for header, value in headers.items():
        assert header_of(ours_call, header) == header_of(reference_call, header) == value
    compare(ours, reference)


def test_calls_delete() -> None:
    with VectorFilesServer() as server:
        ours = delete(
            vector_store_id=STORE_ID,
            file_id=FILE_ID,
            extra_headers=EXTRA_HEADERS,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.calls[-1]
        reference = LITELLM.delete(
            vector_store_id=STORE_ID,
            file_id=FILE_ID,
            extra_headers=EXTRA_HEADERS,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.calls[-1]
    assert ours_call["method"] == "DELETE"
    assert ours_call["method"] == reference_call["method"]
    assert ours_call["route"] == reference_call["route"] == FILE_PATH
    assert ours_call["query"] == reference_call["query"] == {}
    assert header_of(ours_call, "Authorization") == f"Bearer {GOOD_KEY}"
    assert header_of(ours_call, "X-Custom") == "yes"
    compare(ours, reference)


def set_module_globals(
    monkeypatch: pytest.MonkeyPatch, module: ModuleType, url: str, key: str
) -> None:
    monkeypatch.setattr(module, "api_base", url)  # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(module, "api_key", key)  # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(module, "openai_key", key)  # ast-grep-ignore: no-string-mocking


def test_calls_use_module_globals(monkeypatch: pytest.MonkeyPatch) -> None:
    with VectorFilesServer() as server:
        set_module_globals(monkeypatch, betterlite, server.url, GOOD_KEY)
        set_module_globals(monkeypatch, LITELLM_MODULE, server.url, GOOD_KEY)
        ours = retrieve(vector_store_id=STORE_ID, file_id=FILE_ID)
        ours_call = server.calls[-1]
        reference = LITELLM.retrieve(vector_store_id=STORE_ID, file_id=FILE_ID)
        reference_call = server.calls[-1]
    assert ours_call["route"] == reference_call["route"] == FILE_PATH
    assert header_of(ours_call, "Authorization") == f"Bearer {GOOD_KEY}"
    assert header_of(reference_call, "Authorization") == f"Bearer {GOOD_KEY}"
    compare(ours, reference)


def test_calls_refuse_provider() -> None:
    with VectorFilesServer() as server:
        with pytest.raises(APIConnectionError) as ours_caught:
            create(
                vector_store_id=STORE_ID,
                file_id=FILE_ID,
                custom_llm_provider=CUSTOM_PROVIDER,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        with pytest.raises(LITELLM_ERRORS.APIConnectionError):
            LITELLM.create(
                vector_store_id=STORE_ID,
                file_id=FILE_ID,
                custom_llm_provider=CUSTOM_PROVIDER,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        assert server.calls == []
    assert type(ours_caught.value).__name__ == LITELLM_ERRORS.APIConnectionError.__name__


def test_async_returns_coroutines() -> None:
    assert inspect.iscoroutinefunction(acreate)
    assert inspect.iscoroutinefunction(alist)
    assert inspect.iscoroutinefunction(aretrieve)
    assert inspect.iscoroutinefunction(aretrieve_content)
    assert inspect.iscoroutinefunction(aupdate)
    assert inspect.iscoroutinefunction(adelete)
    assert not inspect.iscoroutinefunction(create)
    assert not inspect.iscoroutinefunction(list_files)
    assert not inspect.iscoroutinefunction(retrieve)
    assert not inspect.iscoroutinefunction(retrieve_content)
    assert not inspect.iscoroutinefunction(update)
    assert not inspect.iscoroutinefunction(delete)


def test_async_calls() -> None:
    with VectorFilesServer() as server:
        created, reference_created = asyncio.run(
            awaited_pair(
                acreate(
                    vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url,
                    api_key=GOOD_KEY,
                ),
                LITELLM_ASYNC.acreate(
                    vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url,
                    api_key=GOOD_KEY,
                ),
            )
        )
        page, reference_page = asyncio.run(
            awaited_pair(
                alist(vector_store_id=STORE_ID, api_base=server.url, api_key=GOOD_KEY),
                LITELLM_ASYNC.alist(
                    vector_store_id=STORE_ID, api_base=server.url, api_key=GOOD_KEY
                ),
            )
        )
        found, reference_found = asyncio.run(
            awaited_pair(
                aretrieve(
                    vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url,
                    api_key=GOOD_KEY,
                ),
                LITELLM_ASYNC.aretrieve(
                    vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url,
                    api_key=GOOD_KEY,
                ),
            )
        )
        content, reference_content = asyncio.run(
            awaited_pair(
                aretrieve_content(
                    vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url,
                    api_key=GOOD_KEY,
                ),
                LITELLM_ASYNC.aretrieve_content(
                    vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url,
                    api_key=GOOD_KEY,
                ),
            )
        )
        changed, reference_changed = asyncio.run(
            awaited_pair(
                aupdate(
                    vector_store_id=STORE_ID,
                    file_id=FILE_ID,
                    attributes=UPDATED_ATTRIBUTES,
                    api_base=server.url,
                    api_key=GOOD_KEY,
                ),
                LITELLM_ASYNC.aupdate(
                    vector_store_id=STORE_ID,
                    file_id=FILE_ID,
                    attributes=JSON.validate_python(UPDATED_ATTRIBUTES),
                    api_base=server.url,
                    api_key=GOOD_KEY,
                ),
            )
        )
        removed, reference_removed = asyncio.run(
            awaited_pair(
                adelete(
                    vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url,
                    api_key=GOOD_KEY,
                ),
                LITELLM_ASYNC.adelete(
                    vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url,
                    api_key=GOOD_KEY,
                ),
            )
        )
    compare(created, reference_created)
    compare(page, reference_page)
    compare(found, reference_found)
    compare(content, reference_content)
    compare(changed, reference_changed)
    compare(removed, reference_removed)


def test_async_flagged() -> None:
    with VectorFilesServer() as server:
        plain = create(
            vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url, api_key=GOOD_KEY
        )
        flagged_create = create(
            vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url, api_key=GOOD_KEY,
            acreate=True,
        )
        plain_page = list_files(
            vector_store_id=STORE_ID, api_base=server.url, api_key=GOOD_KEY
        )
        flagged_list = list_files(
            vector_store_id=STORE_ID, api_base=server.url, api_key=GOOD_KEY, alist=True
        )
        plain_read = retrieve(
            vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url, api_key=GOOD_KEY
        )
        flagged_read = retrieve(
            vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url, api_key=GOOD_KEY,
            aretrieve=True,
        )
        plain_content = retrieve_content(
            vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url, api_key=GOOD_KEY
        )
        flagged_content = retrieve_content(
            vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url, api_key=GOOD_KEY,
            aretrieve_content=True,
        )
        plain_update = update(
            vector_store_id=STORE_ID, file_id=FILE_ID, attributes=UPDATED_ATTRIBUTES,
            api_base=server.url, api_key=GOOD_KEY,
        )
        flagged_update = update(
            vector_store_id=STORE_ID, file_id=FILE_ID, attributes=UPDATED_ATTRIBUTES,
            api_base=server.url, api_key=GOOD_KEY, aupdate=True,
        )
        plain_delete = delete(
            vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url, api_key=GOOD_KEY
        )
        flagged_delete = delete(
            vector_store_id=STORE_ID, file_id=FILE_ID, api_base=server.url, api_key=GOOD_KEY,
            adelete=True,
        )
        assert inspect.isawaitable(flagged_create)
        assert inspect.isawaitable(flagged_list)
        assert inspect.isawaitable(flagged_read)
        assert inspect.isawaitable(flagged_content)
        assert inspect.isawaitable(flagged_update)
        assert inspect.isawaitable(flagged_delete)
        created = asyncio.run(settled(flagged_create))
        page = asyncio.run(settled(flagged_list))
        found = asyncio.run(settled(flagged_read))
        content = asyncio.run(settled(flagged_content))
        changed = asyncio.run(settled(flagged_update))
        removed = asyncio.run(settled(flagged_delete))
    assert FILE.validate_python(created) == plain
    assert PAGE.validate_python(page) == plain_page
    assert FILE.validate_python(found) == plain_read
    assert CONTENT.validate_python(content) == plain_content
    assert FILE.validate_python(changed) == plain_update
    assert DELETED.validate_python(removed) == plain_delete


def test_error_keeps_class_cause() -> None:
    with VectorFilesServer() as server:
        with pytest.raises(AuthenticationError) as ours_caught:
            create(
                vector_store_id=STORE_ID,
                file_id=FILE_ID,
                api_base=server.url,
                api_key=BAD_KEY,
            )
        with pytest.raises(LITELLM_ERRORS.AuthenticationError):
            LITELLM.create(
                vector_store_id=STORE_ID,
                file_id=FILE_ID,
                api_base=server.url,
                api_key=BAD_KEY,
            )
    assert type(ours_caught.value).__name__ == LITELLM_ERRORS.AuthenticationError.__name__
    assert ours_caught.value.__cause__ is not None
    assert type(ours_caught.value.__cause__).__name__ == NativeVectorFileError.__name__


def test_error_client_is_refused() -> None:
    with VectorFilesServer() as server:
        with pytest.raises(UnsupportedCallArgument):
            create(
                vector_store_id=STORE_ID,
                file_id=FILE_ID,
                client=object(),
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        assert server.calls == []


def test_error_logger_fn_dropped(caplog: pytest.LogCaptureFixture) -> None:
    with caplog.at_level(logging.WARNING, logger="LiteLLM"), VectorFilesServer() as server:
        ours = create(
            vector_store_id=STORE_ID,
            file_id=FILE_ID,
            litellm_logging_obj=object(),
            logger_fn=print,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.calls[-1]
        reference = LITELLM.create(
            vector_store_id=STORE_ID,
            file_id=FILE_ID,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.calls[-1]
    assert LOGGER_FN_DROPPED in caplog.messages
    assert ours_call["body"] == reference_call["body"]
    compare(ours, reference)
