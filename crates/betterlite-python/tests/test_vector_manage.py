import asyncio
import importlib
import inspect
import json
import logging
import threading
from collections.abc import Awaitable
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import ModuleType
from typing import ClassVar, Final, Protocol
from urllib.parse import parse_qsl, urlsplit

import pytest
from pydantic import JsonValue, TypeAdapter
from vector_server import (
    BAD_KEY,
    GOOD_KEY,
    STORE_ID,
    STORE_PATH,
    RecordedCall,
    create_payload,
    header_of,
)

import betterlite
import betterlite.vector_stores as package
import betterlite.vector_stores.main as package_main
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._endpoints.payload import LOGGER_FN_DROPPED
from betterlite._native import NativeVectorManageError
from betterlite.exceptions import (
    APIConnectionError,
    AuthenticationError,
    BadRequestError,
    NotFoundError,
)
from betterlite.vector_stores._create_search import acreate, asearch, create, search
from betterlite.vector_stores._manage import (
    Managed,
    adelete,
    alist,
    aretrieve,
    aupdate,
    delete,
    retrieve,
    settled,
    update,
)
from betterlite.vector_stores._manage import (
    list as list_call,
)

HOST = "127.0.0.1"
OK = 200
UNAUTHORIZED = 401
NOT_FOUND = 404
CUSTOM_PROVIDER = "anthropic"
AZURE = "azure"
MISSING_ID = "vs_missing"
STORE_URL = f"{STORE_PATH}/{STORE_ID}"
DELETED_OBJECT = "vector_store.deleted"
UPDATED_NAME = "renamed"
AFTER = "vs_cursor_1"
BEFORE = "vs_cursor_2"
LIMIT = 5
ORDER = "asc"
DEFAULT_LIMIT = 20
DEFAULT_ORDER = "desc"
RECORDED_CALLS = 16
EXTRA_HEADERS: Final[dict[str, JsonValue]] = {"X-Custom": "yes"}
EXTRA_QUERY: Final[dict[str, JsonValue]] = {"search": "value"}
EXTRA_BODY: Final[dict[str, JsonValue]] = {"custom_field": "extra"}
EXPORTED: Final[list[str]] = [
    "acreate",
    "adelete",
    "alist",
    "aretrieve",
    "asearch",
    "aupdate",
    "create",
    "delete",
    "list",
    "retrieve",
    "search",
    "update",
]
BODY: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class VectorCall(Protocol):
    def __call__(self, **kwargs: JsonValue) -> dict[str, JsonValue]: ...


class VectorAsyncCall(Protocol):
    def __call__(self, **kwargs: JsonValue) -> Awaitable[dict[str, JsonValue]]: ...


class VectorStores(Protocol):
    def __getattr__(self, name: str) -> VectorCall: ...


class VectorStoresAsync(Protocol):
    def __getattr__(self, name: str) -> VectorAsyncCall: ...


class ExceptionClass(Protocol):
    def __getattr__(self, name: str) -> type[BaseException]: ...


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


class HandleStub:
    ...


LITELLM: VectorStores = importlib.import_module("litellm.vector_stores.main")
LITELLM_ASYNC: VectorStoresAsync = importlib.import_module("litellm.vector_stores.main")
LITELLM_ERRORS: ExceptionClass = importlib.import_module("litellm")
LITELLM_MODULE: Final[ModuleType] = importlib.import_module("litellm")

IMPLEMENTATIONS = (
    acreate,
    adelete,
    alist,
    aretrieve,
    asearch,
    aupdate,
    create,
    delete,
    list_call,
    retrieve,
    search,
    update,
)


def listed_payload() -> dict[str, JsonValue]:
    record = create_payload()
    return {
        "object": "list",
        "data": [record],
        "first_id": STORE_ID,
        "last_id": STORE_ID,
        "has_more": False,
    }


def deleted_payload() -> dict[str, JsonValue]:
    fields: dict[str, JsonValue] = {"deleted": True}
    return {"id": STORE_ID, "object": DELETED_OBJECT, **fields}


def error_payload(message: str) -> dict[str, JsonValue]:
    error: dict[str, JsonValue] = {"message": message, "type": "invalid_request_error"}
    return {"error": error}


# the answer a route gives, before the reply is written back to the caller
def answer_for(method: str, path: str) -> tuple[int, dict[str, JsonValue]]:
    route = urlsplit(path).path
    if route == STORE_PATH:
        return (OK, listed_payload())
    if route != STORE_URL:
        return (NOT_FOUND, error_payload("Vector store not found"))
    if method == "DELETE":
        return (OK, deleted_payload())
    return (OK, create_payload())


class ManageServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[RecordedCall]

    def __init__(self) -> None:
        super().__init__((HOST, 0), _Handler)
        self.calls = []
        _Handler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}/v1"

    def record(
        self, method: str, path: str, headers: dict[str, str], body: dict[str, JsonValue]
    ) -> None:
        self.calls.append({"method": method, "path": path, "headers": headers, "body": body})

    def close(self) -> None:
        self.shutdown()
        self.thread.join()
        self.server_close()


class _Handler(BaseHTTPRequestHandler):
    owner: ClassVar["ManageServer"]

    def do_GET(self) -> None:
        self.answer("GET")

    def do_POST(self) -> None:
        self.answer("POST")

    def do_DELETE(self) -> None:
        self.answer("DELETE")

    def answer(self, method: str) -> None:
        # the Rust transport sends an empty body with Content-Length: 0, where httpx sends none
        declared = self.headers.get("Content-Length")
        length = 0 if declared is None else int(declared)
        raw = b"{}" if length == 0 else self.rfile.read(length)
        body = BODY.validate_json(raw)
        self.owner.record(method, self.path, dict(self.headers.items()), body)
        if self.headers.get("Authorization") != f"Bearer {GOOD_KEY}":
            self.reply(UNAUTHORIZED, error_payload("Incorrect API key provided"))
            return
        status, payload = answer_for(method, self.path)
        self.reply(status, payload)

    def reply(self, status: int, payload: dict[str, JsonValue]) -> None:
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def parameter_rows(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        default: DefaultView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(default.default)))
    return rows


def set_module_globals(
    monkeypatch: pytest.MonkeyPatch, module: ModuleType, url: str, key: str
) -> None:
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(module, "api_base", url)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(module, "api_key", key)
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(module, "openai_key", key)


async def settled_values(*pending: Managed | Awaitable[Managed]) -> tuple[Managed, ...]:
    values = [await settled(item) for item in pending]
    return tuple(values)


def test_signature_matches_litellm() -> None:
    for ours, reference in (
        (retrieve, LITELLM.retrieve),
        (list_call, LITELLM.list),
        (update, LITELLM.update),
        (delete, LITELLM.delete),
        (aretrieve, LITELLM_ASYNC.aretrieve),
        (alist, LITELLM_ASYNC.alist),
        (aupdate, LITELLM_ASYNC.aupdate),
        (adelete, LITELLM_ASYNC.adelete),
    ):
        ours_rows = parameter_rows(inspect.signature(ours))
        reference_rows = parameter_rows(inspect.signature(reference))
        assert ours_rows == reference_rows
    assert inspect.iscoroutinefunction(aretrieve)
    assert inspect.iscoroutinefunction(alist)
    assert inspect.iscoroutinefunction(aupdate)
    assert inspect.iscoroutinefunction(adelete)
    assert not inspect.iscoroutinefunction(retrieve)
    assert not inspect.iscoroutinefunction(list_call)
    assert not inspect.iscoroutinefunction(update)
    assert not inspect.iscoroutinefunction(delete)


def test_calls_retrieve() -> None:
    with closing(ManageServer()) as server:
        ours = retrieve(
            vector_store_id=STORE_ID,
            extra_headers=EXTRA_HEADERS,
            extra_query=EXTRA_QUERY,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.calls[-1]
        reference = LITELLM.retrieve(
            vector_store_id=STORE_ID,
            extra_headers=EXTRA_HEADERS,
            extra_query=EXTRA_QUERY,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.calls[-1]
    assert ours_call["method"] == reference_call["method"] == "GET"
    # retrieve takes no body fields, and extra_query reaches neither request
    assert ours_call["path"] == reference_call["path"] == STORE_URL
    assert ours_call["body"] == reference_call["body"] == {}
    assert header_of(ours_call, "Authorization") == header_of(reference_call, "Authorization")
    assert header_of(ours_call, "Authorization") == f"Bearer {GOOD_KEY}"
    assert header_of(ours_call, "OpenAI-Beta") == header_of(reference_call, "OpenAI-Beta")
    assert header_of(ours_call, "OpenAI-Beta") == "assistants=v2"
    assert header_of(ours_call, "X-Custom") == header_of(reference_call, "X-Custom")
    assert header_of(ours_call, "X-Custom") == "yes"
    assert JSON.validate_python(ours) == JSON.validate_python(reference)


def test_calls_list() -> None:
    with closing(ManageServer()) as server:
        ours = list_call(
            after=AFTER,
            before=BEFORE,
            limit=LIMIT,
            order=ORDER,
            extra_headers=EXTRA_HEADERS,
            extra_query=EXTRA_QUERY,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.calls[-1]
        reference = LITELLM.list(
            after=AFTER,
            before=BEFORE,
            limit=LIMIT,
            order=ORDER,
            extra_headers=EXTRA_HEADERS,
            extra_query=EXTRA_QUERY,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.calls[-1]
    expected: list[tuple[str, str]] = [
        ("after", AFTER),
        ("before", BEFORE),
        ("limit", str(LIMIT)),
        ("order", ORDER),
    ]
    assert ours_call["method"] == reference_call["method"] == "GET"
    assert urlsplit(ours_call["path"]).path == urlsplit(reference_call["path"]).path
    assert urlsplit(ours_call["path"]).path == STORE_PATH
    # the four cursors travel in this order, and only the ones the caller set
    ours_query = parse_qsl(urlsplit(ours_call["path"]).query)
    assert ours_query == parse_qsl(urlsplit(reference_call["path"]).query) == expected
    assert ours_call["body"] == reference_call["body"] == {}
    assert header_of(ours_call, "Authorization") == f"Bearer {GOOD_KEY}"
    assert JSON.validate_python(ours) == JSON.validate_python(reference)


def test_calls_list_defaults() -> None:
    with closing(ManageServer()) as server:
        ours = list_call(api_base=server.url, api_key=GOOD_KEY)
        ours_call = server.calls[-1]
        reference = LITELLM.list(api_base=server.url, api_key=GOOD_KEY)
        reference_call = server.calls[-1]
    expected: list[tuple[str, str]] = [
        ("limit", str(DEFAULT_LIMIT)),
        ("order", DEFAULT_ORDER),
    ]
    assert ours_call["path"] == reference_call["path"]
    ours_query = parse_qsl(urlsplit(ours_call["path"]).query)
    assert ours_query == parse_qsl(urlsplit(reference_call["path"]).query) == expected
    assert JSON.validate_python(ours) == JSON.validate_python(reference)


def test_calls_update() -> None:
    with closing(ManageServer()) as server:
        ours = update(
            vector_store_id=STORE_ID,
            name=UPDATED_NAME,
            metadata={"team": "manage"},
            extra_headers=EXTRA_HEADERS,
            extra_body=EXTRA_BODY,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.calls[-1]
        reference = LITELLM.update(
            vector_store_id=STORE_ID,
            name=UPDATED_NAME,
            metadata={"team": "manage"},
            extra_headers=EXTRA_HEADERS,
            extra_body=EXTRA_BODY,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.calls[-1]
    expected = {"name": UPDATED_NAME, "metadata": {"team": "manage"}} | EXTRA_BODY
    assert ours_call["method"] == reference_call["method"] == "POST"
    assert ours_call["path"] == reference_call["path"] == STORE_URL
    assert ours_call["body"] == reference_call["body"] == expected
    assert header_of(ours_call, "Content-Type") == header_of(reference_call, "Content-Type")
    assert "expires_after" not in ours_call["body"]
    assert JSON.validate_python(ours) == JSON.validate_python(reference)


def test_update_sends_empty_meta() -> None:
    with closing(ManageServer()) as server:
        ours = update(vector_store_id=STORE_ID, metadata={}, api_base=server.url, api_key=GOOD_KEY)
        ours_call = server.calls[-1]
        reference = LITELLM.update(
            vector_store_id=STORE_ID, metadata={}, api_base=server.url, api_key=GOOD_KEY
        )
        reference_call = server.calls[-1]
    assert ours_call["body"] == reference_call["body"] == {"metadata": {}}
    assert JSON.validate_python(ours) == JSON.validate_python(reference)


def test_calls_delete() -> None:
    with closing(ManageServer()) as server:
        ours = delete(
            vector_store_id=STORE_ID,
            extra_headers=EXTRA_HEADERS,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.calls[-1]
        reference = LITELLM.delete(
            vector_store_id=STORE_ID,
            extra_headers=EXTRA_HEADERS,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.calls[-1]
    assert ours_call["method"] == reference_call["method"] == "DELETE"
    assert ours_call["path"] == reference_call["path"] == STORE_URL
    assert ours_call["body"] == reference_call["body"] == {}
    assert header_of(ours_call, "Authorization") == f"Bearer {GOOD_KEY}"
    assert JSON.validate_python(ours) == JSON.validate_python(reference)
    assert JSON.validate_python(ours) == JSON.validate_python(deleted_payload())


def test_calls_async() -> None:
    with closing(ManageServer()) as server:
        plain_retrieve = retrieve(vector_store_id=STORE_ID, api_base=server.url, api_key=GOOD_KEY)
        plain_list = list_call(api_base=server.url, api_key=GOOD_KEY)
        plain_update = update(
            vector_store_id=STORE_ID, name=UPDATED_NAME, api_base=server.url, api_key=GOOD_KEY
        )
        plain_delete = delete(vector_store_id=STORE_ID, api_base=server.url, api_key=GOOD_KEY)
        plain = (plain_retrieve, plain_list, plain_update, plain_delete)
        flagged = (
            retrieve(
                vector_store_id=STORE_ID, api_base=server.url, api_key=GOOD_KEY, aretrieve=True
            ),
            list_call(api_base=server.url, api_key=GOOD_KEY, alist=True),
            update(
                vector_store_id=STORE_ID,
                name=UPDATED_NAME,
                api_base=server.url,
                api_key=GOOD_KEY,
                aupdate=True,
            ),
            delete(
                vector_store_id=STORE_ID, api_base=server.url, api_key=GOOD_KEY, adelete=True
            ),
        )
        awaited = (
            aretrieve(vector_store_id=STORE_ID, api_base=server.url, api_key=GOOD_KEY),
            LITELLM_ASYNC.aretrieve(
                vector_store_id=STORE_ID, api_base=server.url, api_key=GOOD_KEY
            ),
            alist(api_base=server.url, api_key=GOOD_KEY),
            LITELLM_ASYNC.alist(api_base=server.url, api_key=GOOD_KEY),
            aupdate(
                vector_store_id=STORE_ID, name=UPDATED_NAME, api_base=server.url, api_key=GOOD_KEY
            ),
            LITELLM_ASYNC.aupdate(
                vector_store_id=STORE_ID, name=UPDATED_NAME, api_base=server.url, api_key=GOOD_KEY
            ),
            adelete(vector_store_id=STORE_ID, api_base=server.url, api_key=GOOD_KEY),
            LITELLM_ASYNC.adelete(
                vector_store_id=STORE_ID, api_base=server.url, api_key=GOOD_KEY
            ),
        )
        assert not any(inspect.isawaitable(value) for value in plain)
        assert all(inspect.isawaitable(pending) for pending in flagged)
        results = asyncio.run(settled_values(*flagged, *awaited))
    assert len(server.calls) == RECORDED_CALLS
    flagged_values = results[: len(flagged)]
    awaited_values = results[len(flagged) :]
    for value, plain_value in zip(flagged_values, plain, strict=True):
        assert JSON.validate_python(value) == JSON.validate_python(plain_value)
    paired = zip(awaited_values[::2], awaited_values[1::2], strict=True)
    for ours_value, reference_value in paired:
        assert JSON.validate_python(ours_value) == JSON.validate_python(reference_value)


def test_calls_use_module_globals(monkeypatch: pytest.MonkeyPatch) -> None:
    with closing(ManageServer()) as server:
        set_module_globals(monkeypatch, betterlite, server.url, GOOD_KEY)
        set_module_globals(monkeypatch, LITELLM_MODULE, server.url, GOOD_KEY)
        ours_retrieve = retrieve(vector_store_id=STORE_ID)
        ours_retrieve_call = server.calls[-1]
        reference_retrieve = LITELLM.retrieve(vector_store_id=STORE_ID)
        reference_retrieve_call = server.calls[-1]
        ours_update = update(vector_store_id=STORE_ID, name=UPDATED_NAME)
        ours_update_call = server.calls[-1]
        reference_update = LITELLM.update(vector_store_id=STORE_ID, name=UPDATED_NAME)
        reference_update_call = server.calls[-1]
    assert ours_retrieve_call["path"] == reference_retrieve_call["path"]
    assert ours_retrieve_call["body"] == reference_retrieve_call["body"]
    assert header_of(ours_retrieve_call, "Authorization") == f"Bearer {GOOD_KEY}"
    assert ours_update_call["path"] == reference_update_call["path"]
    assert ours_update_call["body"] == reference_update_call["body"]
    assert header_of(ours_update_call, "Authorization") == f"Bearer {GOOD_KEY}"
    assert JSON.validate_python(ours_retrieve) == JSON.validate_python(reference_retrieve)
    assert JSON.validate_python(ours_update) == JSON.validate_python(reference_update)


def test_calls_drop_logging(caplog: pytest.LogCaptureFixture) -> None:
    with caplog.at_level(logging.WARNING, logger="LiteLLM"), closing(ManageServer()) as server:
        ours_values: list[Managed | Awaitable[Managed]] = []
        ours_calls: list[RecordedCall] = []
        for call in (retrieve, delete):
            ours_values.append(
                call(
                    vector_store_id=STORE_ID,
                    litellm_logging_obj=object(),
                    logger_fn=print,
                    api_base=server.url,
                    api_key=GOOD_KEY,
                )
            )
            ours_calls.append(server.calls[-1])
        reference_values: list[dict[str, JsonValue]] = []
        reference_calls: list[RecordedCall] = []
        for reference in (LITELLM.retrieve, LITELLM.delete):
            reference_values.append(
                reference(vector_store_id=STORE_ID, api_base=server.url, api_key=GOOD_KEY)
            )
            reference_calls.append(server.calls[-1])
    assert LOGGER_FN_DROPPED in caplog.messages
    for ours_value, reference_value in zip(ours_values, reference_values, strict=True):
        assert JSON.validate_python(ours_value) == JSON.validate_python(reference_value)
    for ours_call, reference_call in zip(ours_calls, reference_calls, strict=True):
        assert ours_call["path"] == reference_call["path"]
        assert header_of(ours_call, "Authorization") == header_of(reference_call, "Authorization")


# a provider betterlite cannot serve: both sides raise the same class, and no request leaves
def test_calls_refuse_provider() -> None:
    with closing(ManageServer()) as server:
        with pytest.raises(APIConnectionError) as ours_caught:
            retrieve(
                vector_store_id=STORE_ID,
                custom_llm_provider=CUSTOM_PROVIDER,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        with pytest.raises(LITELLM_ERRORS.APIConnectionError):
            LITELLM.retrieve(
                vector_store_id=STORE_ID,
                custom_llm_provider=CUSTOM_PROVIDER,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        with pytest.raises(APIConnectionError):
            list_call(
                custom_llm_provider=CUSTOM_PROVIDER,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        with pytest.raises(LITELLM_ERRORS.APIConnectionError):
            LITELLM.list(
                custom_llm_provider=CUSTOM_PROVIDER,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        with pytest.raises(APIConnectionError):
            update(
                vector_store_id=STORE_ID,
                name=UPDATED_NAME,
                custom_llm_provider=CUSTOM_PROVIDER,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        with pytest.raises(LITELLM_ERRORS.APIConnectionError):
            LITELLM.update(
                vector_store_id=STORE_ID,
                name=UPDATED_NAME,
                custom_llm_provider=CUSTOM_PROVIDER,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        with pytest.raises(APIConnectionError):
            delete(
                vector_store_id=STORE_ID,
                custom_llm_provider=CUSTOM_PROVIDER,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        with pytest.raises(LITELLM_ERRORS.APIConnectionError):
            LITELLM.delete(
                vector_store_id=STORE_ID,
                custom_llm_provider=CUSTOM_PROVIDER,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        assert server.calls == []
    assert type(ours_caught.value).__name__ == LITELLM_ERRORS.APIConnectionError.__name__


# azure is a tag LiteLLM serves and this build does not: the refusal is deliberate
def test_calls_refuse_azure() -> None:
    with closing(ManageServer()) as server:
        with pytest.raises(APIConnectionError):
            retrieve(
                vector_store_id=STORE_ID,
                custom_llm_provider=AZURE,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        with pytest.raises(APIConnectionError):
            list_call(
                custom_llm_provider=AZURE,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        with pytest.raises(BadRequestError):
            update(
                vector_store_id=STORE_ID,
                name=UPDATED_NAME,
                custom_llm_provider=AZURE,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        with pytest.raises(BadRequestError):
            delete(
                vector_store_id=STORE_ID,
                custom_llm_provider=AZURE,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        assert server.calls == []


def test_error_keeps_class() -> None:
    authentication_class = LITELLM_ERRORS.AuthenticationError
    not_found_class = LITELLM_ERRORS.NotFoundError
    with closing(ManageServer()) as server:
        # a GET answers with the reply body, and retrieve keeps only its declared fields
        reference_error = LITELLM.retrieve(
            vector_store_id=STORE_ID, api_base=server.url, api_key=BAD_KEY
        )
        assert reference_error == error_payload("Incorrect API key provided")
        assert retrieve(vector_store_id=STORE_ID, api_base=server.url, api_key=BAD_KEY) == {}
        reference_list = LITELLM.list(api_base=server.url, api_key=BAD_KEY)
        assert list_call(api_base=server.url, api_key=BAD_KEY) == reference_list
        with pytest.raises(AuthenticationError) as authenticated_update:
            update(vector_store_id=STORE_ID, api_base=server.url, api_key=BAD_KEY)
        with pytest.raises(authentication_class):
            LITELLM.update(vector_store_id=STORE_ID, api_base=server.url, api_key=BAD_KEY)
        with pytest.raises(NotFoundError) as missing_delete:
            delete(vector_store_id=MISSING_ID, api_base=server.url, api_key=GOOD_KEY)
        with pytest.raises(not_found_class):
            LITELLM.delete(vector_store_id=MISSING_ID, api_base=server.url, api_key=GOOD_KEY)
    caught = (
        (authenticated_update, authentication_class, UNAUTHORIZED),
        (missing_delete, not_found_class, NOT_FOUND),
    )
    for ours_caught, reference_class, status in caught:
        assert type(ours_caught.value).__name__ == reference_class.__name__
        assert ours_caught.value.status_code == status
        match ours_caught.value.__cause__:
            case NativeVectorManageError():
                pass
            case object():
                raise AssertionError("the provider error carries no native cause")


def test_error_refuses_client() -> None:
    with pytest.raises(UnsupportedCallArgument, match="client"):
        retrieve(vector_store_id=STORE_ID, client=HandleStub())
    with pytest.raises(UnsupportedCallArgument, match="client"):
        asyncio.run(aretrieve(vector_store_id=STORE_ID, client=HandleStub()))
    with pytest.raises(UnsupportedCallArgument, match="client"):
        update(vector_store_id=STORE_ID, client=HandleStub())
    with pytest.raises(UnsupportedCallArgument, match="shared_session"):
        delete(vector_store_id=STORE_ID, shared_session=HandleStub())


def test_package_exports() -> None:
    package_values = (
        package.acreate,
        package.adelete,
        package.alist,
        package.aretrieve,
        package.asearch,
        package.aupdate,
        package.create,
        package.delete,
        package.list,
        package.retrieve,
        package.search,
        package.update,
    )
    main_values = (
        package_main.acreate,
        package_main.adelete,
        package_main.alist,
        package_main.aretrieve,
        package_main.asearch,
        package_main.aupdate,
        package_main.create,
        package_main.delete,
        package_main.list,
        package_main.retrieve,
        package_main.search,
        package_main.update,
    )
    assert package.__all__ == EXPORTED
    assert package_main.__all__ == EXPORTED
    assert package_values == IMPLEMENTATIONS
    assert main_values == IMPLEMENTATIONS
