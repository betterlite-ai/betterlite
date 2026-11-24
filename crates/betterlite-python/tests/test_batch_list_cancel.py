import asyncio
import inspect
import threading
from collections.abc import Awaitable
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, Self
from urllib.parse import parse_qsl, urlsplit

import pytest
from batch_server import (
    BATCH_ID,
    BATCH_PATH,
    CREATED_AT,
    ENDPOINT,
    GOOD_KEY,
    HOST,
    INPUT_FILE_ID,
    LITELLM,
    METADATA,
    MISSING_ID,
    MODEL,
    NOT_FOUND,
    OK,
    UNAUTHORIZED,
    BatchDump,
    parameter_rows,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._native import NativeBatchListError
from betterlite.batches import main as batch_main
from betterlite.batches._create_retrieve import LiteLLMBatch
from betterlite.batches._list_cancel import (
    BatchListPage,
    acancel_batch,
    alist_batches,
    cancel_batch,
    list_batches,
)
from betterlite.exceptions import APIConnectionError, AuthenticationError, NotFoundError

CALLS_PER_COMPARISON = 2
AFTER = "batch_prev"
LIMIT = 2
CANCELLED_AT = 1710000100
EXTRA_HEADERS: Final[dict[str, str]] = {"x-test": "1"}
EXTRA_BODY: Final[dict[str, str]] = {"note": "n"}

BODY: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
HIDDEN: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class RecordedCall(TypedDict):
    method: str
    path: str
    query: dict[str, str]
    headers: dict[str, str]
    body: dict[str, JsonValue]


def batch_object(batch_id: str, status: str) -> dict[str, JsonValue]:
    return {
        "id": batch_id,
        "completion_window": "24h",
        "created_at": CREATED_AT,
        "endpoint": ENDPOINT,
        "input_file_id": INPUT_FILE_ID,
        "object": "batch",
        "status": status,
        "metadata": {"team": "x"},
        "model": MODEL,
        "request_counts": {"completed": 0, "failed": 0, "total": 1},
    }


def page_object() -> dict[str, JsonValue]:
    return {
        "object": "list",
        "data": [batch_object(BATCH_ID, "in_progress")],
        "first_id": BATCH_ID,
        "last_id": BATCH_ID,
        "has_more": False,
    }


def cancelled_object() -> dict[str, JsonValue]:
    batch = batch_object(BATCH_ID, "cancelled")
    batch["cancelled_at"] = CANCELLED_AT
    return batch


class ListServer(ThreadingHTTPServer):
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

    def record(self, call: RecordedCall) -> None:
        self.calls.append(call)

    @override
    def __enter__(self) -> Self:
        return self

    @override
    def __exit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class _Handler(BaseHTTPRequestHandler):
    owner: ClassVar[ListServer]

    def do_GET(self) -> None:
        parts = urlsplit(self.path)
        self.record_call("GET", parts.path, parts.query, {})
        if not self.authorized():
            return
        if parts.path != BATCH_PATH:
            self.reply_error(NOT_FOUND, f"no batch list at {parts.path}")
            return
        self.reply(OK, page_object())

    def do_POST(self) -> None:
        parts = urlsplit(self.path)
        body = self.read_body()
        self.record_call("POST", parts.path, parts.query, body)
        if not self.authorized():
            return
        if parts.path == f"{BATCH_PATH}/{MISSING_ID}/cancel":
            self.reply_error(NOT_FOUND, "no such batch")
            return
        if parts.path != f"{BATCH_PATH}/{BATCH_ID}/cancel":
            self.reply_error(NOT_FOUND, f"no cancel at {parts.path}")
            return
        self.reply(OK, cancelled_object())

    def record_call(
        self, method: str, path: str, query: str, body: dict[str, JsonValue]
    ) -> None:
        self.owner.record(
            {
                "method": method,
                "path": path,
                "query": dict(parse_qsl(query)),
                "headers": dict(self.headers.items()),
                "body": body,
            }
        )

    def authorized(self) -> bool:
        if self.headers.get("Authorization") == f"Bearer {GOOD_KEY}":
            return True
        self.reply_error(UNAUTHORIZED, "invalid api key")
        return False

    def read_body(self) -> dict[str, JsonValue]:
        declared = self.headers.get("Content-Length")
        if declared is None:
            return {}
        raw = self.rfile.read(int(declared))
        if not raw:
            return {}
        return BODY.validate_json(raw)

    def reply_error(self, status: int, message: str) -> None:
        payload: dict[str, JsonValue] = {
            "error": {"message": message, "type": "invalid_request_error"},
        }
        self.reply(status, payload)

    def reply(self, status: int, payload: dict[str, JsonValue]) -> None:
        encoded = BODY.dump_json(payload)
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def made_page(result: BatchListPage | Awaitable[BatchListPage]) -> BatchListPage:
    match result:
        case BatchListPage():
            return result
        case Awaitable():
            raise AssertionError("the call answered with a coroutine")


async def awaited_page(pending: Awaitable[BatchListPage]) -> BatchListPage:
    page = await pending
    assert page.data[0].id == BATCH_ID
    return page


def made_batch(result: LiteLLMBatch | Awaitable[LiteLLMBatch]) -> LiteLLMBatch:
    match result:
        case LiteLLMBatch():
            return result
        case Awaitable():
            raise AssertionError("the call answered with a coroutine")


async def awaited_batch(pending: Awaitable[LiteLLMBatch]) -> LiteLLMBatch:
    batch = await pending
    assert batch.id == BATCH_ID
    return batch


def hidden_of(response: LiteLLMBatch | BatchDump) -> dict[str, JsonValue]:
    private: dict[str, JsonValue] | None = response.__pydantic_private__
    assert private is not None
    return HIDDEN.validate_python(private["_hidden_params"])


def native_cause(error: BaseException) -> NativeBatchListError:
    cause = error.__cause__
    match cause:
        case NativeBatchListError():
            return cause
        case object():
            raise AssertionError("the exception does not carry the native error as its cause")


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(list_batches)) == parameter_rows(
        inspect.signature(LITELLM.list_batches)
    )
    assert parameter_rows(inspect.signature(alist_batches)) == parameter_rows(
        inspect.signature(LITELLM.alist_batches)
    )
    assert parameter_rows(inspect.signature(cancel_batch)) == parameter_rows(
        inspect.signature(LITELLM.cancel_batch)
    )
    assert parameter_rows(inspect.signature(acancel_batch)) == parameter_rows(
        inspect.signature(LITELLM.acancel_batch)
    )
    # alist_batches declares metadata where list_batches has none
    assert "metadata" in inspect.signature(alist_batches).parameters
    assert "metadata" not in inspect.signature(list_batches).parameters
    assert not inspect.iscoroutinefunction(list_batches)
    assert not inspect.iscoroutinefunction(cancel_batch)
    assert inspect.iscoroutinefunction(alist_batches)
    assert inspect.iscoroutinefunction(acancel_batch)


def test_page_matches_litellm() -> None:
    with ListServer() as server:
        reference = LITELLM.list_batches(
            after=AFTER, limit=LIMIT, api_base=server.url, api_key=GOOD_KEY
        )
        made = made_page(
            list_batches(after=AFTER, limit=LIMIT, api_base=server.url, api_key=GOOD_KEY)
        )
        assert len(server.calls) == CALLS_PER_COMPARISON
        reference_call, ours_call = server.calls
    assert made.model_dump() == reference.model_dump()
    assert ours_call["method"] == "GET"
    assert ours_call["path"] == BATCH_PATH
    assert dict(ours_call["query"]) == dict(reference_call["query"])
    assert dict(ours_call["query"]) == {"after": AFTER, "limit": str(LIMIT)}


def test_page_defaults() -> None:
    with ListServer() as server:
        reference = LITELLM.list_batches(api_base=server.url, api_key=GOOD_KEY)
        made = made_page(list_batches(api_base=server.url, api_key=GOOD_KEY))
        nulled = LITELLM.list_batches(
            after=None, limit=None, api_base=server.url, api_key=GOOD_KEY
        )
        null_made = made_page(
            list_batches(after=None, limit=None, api_base=server.url, api_key=GOOD_KEY)
        )
        zero = LITELLM.list_batches(limit=0, api_base=server.url, api_key=GOOD_KEY)
        zero_made = made_page(list_batches(limit=0, api_base=server.url, api_key=GOOD_KEY))
        calls = server.calls
    assert [made.model_dump(), null_made.model_dump(), zero_made.model_dump()] == [
        reference.model_dump(),
        nulled.model_dump(),
        zero.model_dump(),
    ]
    assert [dict(call["query"]) for call in calls[1::2]] == [{}, {}, {"limit": "0"}]
    assert [dict(call["query"]) for call in calls[::2]] == [{}, {}, {"limit": "0"}]


def test_page_drops_metadata() -> None:
    with ListServer() as server:
        reference = LITELLM.list_batches(api_base=server.url, api_key=GOOD_KEY)
        # list_batches has no metadata parameter, so the kwarg arrives in **kwargs
        made = made_page(
            list_batches(metadata=dict(METADATA), api_base=server.url, api_key=GOOD_KEY)
        )
        listed = asyncio.run(
            awaited_page(
                alist_batches(metadata=dict(METADATA), api_base=server.url, api_key=GOOD_KEY)
            )
        )
        assert len(server.calls) == CALLS_PER_COMPARISON + 1
        reference_call, ours_call, listed_call = server.calls
    assert made.model_dump() == reference.model_dump()
    assert listed.model_dump() == reference.model_dump()
    assert dict(ours_call["query"]) == dict(reference_call["query"])
    assert dict(listed_call["query"]) == dict(reference_call["query"]) == {}


def test_page_drops_extras() -> None:
    with ListServer() as server:
        reference = LITELLM.list_batches(
            extra_headers=dict(EXTRA_HEADERS),
            extra_body=dict(EXTRA_BODY),
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        made = made_page(
            list_batches(
                extra_headers=EXTRA_HEADERS,
                extra_body=EXTRA_BODY,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        assert len(server.calls) == CALLS_PER_COMPARISON
        reference_call, ours_call = server.calls
    assert made.model_dump() == reference.model_dump()
    assert dict(ours_call["query"]) == dict(reference_call["query"])
    assert "x-test" not in {name.lower() for name in ours_call["headers"]}
    assert "x-test" not in {name.lower() for name in reference_call["headers"]}


def test_page_coroutines() -> None:
    with ListServer() as server:
        reference = LITELLM.list_batches(api_base=server.url, api_key=GOOD_KEY)
        flagged = list_batches(api_base=server.url, api_key=GOOD_KEY, alist_batches=True)
        assert inspect.isawaitable(flagged)
        flagged_page = asyncio.run(awaited_page(flagged))
        direct = alist_batches(api_base=server.url, api_key=GOOD_KEY)
        assert inspect.isawaitable(direct)
        direct_page = asyncio.run(awaited_page(direct))
    assert flagged_page.model_dump() == reference.model_dump()
    assert direct_page.model_dump() == reference.model_dump()


def test_page_error_class() -> None:
    with ListServer() as server, pytest.raises(AuthenticationError) as caught:
        list_batches(api_base=server.url, api_key="sk-wrong")
    cause = native_cause(caught.value)
    assert cause is caught.value.__cause__
    assert caught.value.llm_provider == "openai"


def test_cancel_matches_litellm() -> None:
    with ListServer() as server:
        reference = LITELLM.cancel_batch(
            batch_id=BATCH_ID, api_base=server.url, api_key=GOOD_KEY
        )
        made = made_batch(cancel_batch(batch_id=BATCH_ID, api_base=server.url, api_key=GOOD_KEY))
        assert len(server.calls) == CALLS_PER_COMPARISON
        reference_call, ours_call = server.calls
    assert made.model_dump() == reference.model_dump()
    assert ours_call["method"] == "POST"
    assert ours_call["path"] == f"{BATCH_PATH}/{BATCH_ID}/cancel"
    assert ours_call["body"] == reference_call["body"] == {}


def test_cancel_passes_the_model() -> None:
    with ListServer() as server:
        reference = LITELLM.cancel_batch(
            batch_id=BATCH_ID, model=MODEL, api_base=server.url, api_key=GOOD_KEY
        )
        made = made_batch(
            cancel_batch(
                batch_id=BATCH_ID, model=MODEL, api_base=server.url, api_key=GOOD_KEY
            )
        )
        assert len(server.calls) == CALLS_PER_COMPARISON
    assert made.model_dump() == reference.model_dump()
    hidden = hidden_of(made)
    assert hidden["llm_provider"] == "openai"
    assert hidden["model_id"] is None
    assert hidden["litellm_model_name"] == MODEL
    assert hidden["api_base"] == server.url


def test_cancel_infers_provider() -> None:
    # LiteLLM sends azure/command-r to the openai branch, so the failure names openai
    with pytest.raises(APIConnectionError) as caught:
        cancel_batch(
            batch_id=BATCH_ID,
            model="azure/command-r",
            custom_llm_provider="azure",
            api_base="http://127.0.0.1:1/v1",
            api_key=GOOD_KEY,
        )
    assert caught.value.llm_provider == "openai"


def test_cancel_coroutines() -> None:
    with ListServer() as server:
        flagged = cancel_batch(
            batch_id=BATCH_ID, api_base=server.url, api_key=GOOD_KEY, acancel_batch=True
        )
        direct = acancel_batch(batch_id=BATCH_ID, api_base=server.url, api_key=GOOD_KEY)
        assert inspect.isawaitable(flagged)
        assert inspect.isawaitable(direct)
        through_flag = asyncio.run(awaited_batch(flagged))
        through_direct = asyncio.run(awaited_batch(direct))
    assert through_flag == through_direct
    assert through_flag.status == "cancelled"


def test_cancel_sends_extras() -> None:
    with ListServer() as server:
        reference = LITELLM.cancel_batch(
            batch_id=BATCH_ID,
            extra_headers=dict(EXTRA_HEADERS),
            extra_body=dict(EXTRA_BODY),
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        made = made_batch(
            cancel_batch(
                batch_id=BATCH_ID,
                extra_headers=EXTRA_HEADERS,
                extra_body=EXTRA_BODY,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        assert len(server.calls) == CALLS_PER_COMPARISON
        reference_call, ours_call = server.calls
    assert made.model_dump() == reference.model_dump()
    assert ours_call["body"] == reference_call["body"] == EXTRA_BODY
    assert ours_call["headers"].get("x-test") == "1"


def test_cancel_drops_metadata() -> None:
    with ListServer() as server:
        reference = LITELLM.cancel_batch(
            batch_id=BATCH_ID, metadata=dict(METADATA), api_base=server.url, api_key=GOOD_KEY
        )
        made = made_batch(
            cancel_batch(
                batch_id=BATCH_ID,
                metadata=dict(METADATA),
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        assert len(server.calls) == CALLS_PER_COMPARISON
        reference_call, ours_call = server.calls
    assert made.model_dump() == reference.model_dump()
    assert ours_call["body"] == reference_call["body"] == {}


def test_cancel_error_class() -> None:
    with ListServer() as server, pytest.raises(NotFoundError) as caught:
        cancel_batch(batch_id=MISSING_ID, api_base=server.url, api_key=GOOD_KEY)
    cause = native_cause(caught.value)
    assert cause is caught.value.__cause__
    assert caught.value.status_code == NOT_FOUND


def test_cancel_refuses_client() -> None:
    with pytest.raises(UnsupportedCallArgument) as refused:
        cancel_batch(batch_id=BATCH_ID, client=object())
    assert refused.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as session_refused:
        cancel_batch(batch_id=BATCH_ID, shared_session=object())
    assert session_refused.value.argument == "shared_session"


def test_package_exports() -> None:
    expected = [
        "BatchPath",
        "LiteLLMBatch",
        "acancel_batch",
        "acreate_batch",
        "alist_batches",
        "aretrieve_batch",
        "cancel_batch",
        "create_batch",
        "list_batches",
        "retrieve_batch",
    ]
    assert batch_main.__all__ == expected
    public = sorted(name for name in dir(batch_main) if not name.startswith("_"))
    assert public == sorted(expected)
