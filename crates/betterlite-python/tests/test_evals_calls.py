import asyncio
import importlib
import inspect
import json
import threading
from collections.abc import Awaitable, Callable, Coroutine
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, Protocol, Self, TypedDict

import pytest
from pydantic import BaseModel, JsonValue
from typing_extensions import override

import betterlite.exceptions as our_errors
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite.evals._evals import (
    acancel_eval,
    acreate_eval,
    adelete_eval,
    aget_eval,
    alist_evals,
    aupdate_eval,
    cancel_eval,
    create_eval,
    delete_eval,
    get_eval,
    list_evals,
    update_eval,
)
from betterlite.exceptions import APIConnectionError, BadRequestError, NotFoundError
from betterlite.types.llms.openai_evals import DataSourceConfig, GraderConfig

HOST: Final = "127.0.0.1"
OK: Final = 200
BAD_REQUEST: Final = 400
NOT_FOUND: Final = 404
SERVER_ERROR: Final = 500
GOOD_KEY: Final = "sk-test"
EVAL_ID: Final = "eval_1"
EVALS_PATH: Final = "/v1/evals"
CANCEL_SUFFIX: Final = "/cancel"
CUSTOM_HEADER: Final = "x-eval-test"
CUSTOM_VALUE: Final = "on"
TIMEOUT: Final = 30.0
ANTHROPIC: Final = "anthropic"
OPENAI: Final = "openai"
JSON_TYPE: Final = "application/json"
BAD_MESSAGE: Final = "the eval request is invalid"
MISSING_MESSAGE: Final = "eval not found"
NAME: Final = "probe"

DATA_SOURCE_CONFIG: Final[DataSourceConfig] = {
    "type": "custom",
    "item_schema": {"type": "object"},
}
TESTING_CRITERIA: Final[list[GraderConfig]] = [{"type": "custom", "function_id": "fn-1"}]
METADATA: Final[dict[str, JsonValue]] = {"team": "evals"}
UPDATE_METADATA: Final[dict[str, JsonValue]] = {"headers": "internal", "team": "evals"}
INTERNAL_METADATA: Final[dict[str, JsonValue]] = {"headers": "internal"}
EXTRA_BODY: Final[dict[str, JsonValue]] = {"note": "n"}
HEADERS: Final[dict[str, str]] = {CUSTOM_HEADER: CUSTOM_VALUE}
QUERY: Final[dict[str, JsonValue]] = {"limit": 9, "after": "eval_x", "unknown": "x"}
DUPLICATE: Final[dict[str, JsonValue]] = {"name": "second"}

BAD_BODY: Final[dict[str, JsonValue]] = {
    "type": "error",
    "error": {"type": "invalid_request_error", "message": BAD_MESSAGE},
}
MISSING_BODY: Final[dict[str, JsonValue]] = {
    "type": "error",
    "error": {"type": "not_found_error", "message": MISSING_MESSAGE},
}
DELETED_BODY: Final[dict[str, JsonValue]] = {
    "eval_id": EVAL_ID,
    "object": "eval.deleted",
    "deleted": True,
}
CANCELLED_BODY: Final[dict[str, JsonValue]] = {
    "id": EVAL_ID,
    "object": "eval",
    "status": "cancelled",
}

# the calls both packages make, in order: create, list, get, update, delete, cancel
OPERATIONS: Final[tuple[tuple[str, str], ...]] = (
    ("POST", EVALS_PATH),
    ("GET", f"{EVALS_PATH}?limit=5"),
    ("GET", f"{EVALS_PATH}/{EVAL_ID}"),
    ("POST", f"{EVALS_PATH}/{EVAL_ID}"),
    ("DELETE", f"{EVALS_PATH}/{EVAL_ID}"),
    ("POST", f"{EVALS_PATH}/{EVAL_ID}{CANCEL_SUFFIX}"),
)


class Recorded(TypedDict):
    method: str
    path: str
    headers: dict[str, str]
    body: bytes


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


class RefCall(Protocol):
    def __call__(self, **values: object) -> BaseModel: ...


class RefAsyncCall(Protocol):
    def __call__(self, **values: object) -> Coroutine[object, object, BaseModel]: ...


class RefModule(Protocol):
    def __getattr__(self, name: str) -> RefCall: ...


class RefAsyncModule(Protocol):
    def __getattr__(self, name: str) -> RefAsyncCall: ...


class ReferenceError(Exception):
    status_code: int


class RefErrorModule(Protocol):
    def __getattr__(self, name: str) -> type[ReferenceError]: ...


LITELLM: RefModule = importlib.import_module("litellm")
LITELLM_ASYNC: RefAsyncModule = importlib.import_module("litellm")
LITELLM_ERRORS: RefErrorModule = importlib.import_module("litellm.exceptions")


class EvalsServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[Recorded]
    failing: bool
    missing: bool

    def __init__(self, *, failing: bool = False, missing: bool = False) -> None:
        super().__init__((HOST, 0), _Handler)
        self.calls = []
        self.failing = failing
        self.missing = missing
        _Handler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}"

    def record(self, method: str, path: str, headers: dict[str, str], body: bytes) -> None:
        self.calls.append({"method": method, "path": path, "headers": headers, "body": body})

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
    owner: ClassVar[EvalsServer]

    def do_POST(self) -> None:
        self.owner.record("POST", self.path, self.headers_dict(), self.body_bytes())
        if self.refused():
            return
        path = self.path.partition("?")[0]
        if path == EVALS_PATH:
            self.reply(OK, eval_body("created"))
            return
        if path.endswith(CANCEL_SUFFIX):
            self.reply(OK, CANCELLED_BODY)
            return
        self.reply(OK, eval_body("updated"))

    def do_GET(self) -> None:
        self.owner.record("GET", self.path, self.headers_dict(), self.body_bytes())
        if self.refused():
            return
        if self.path.partition("?")[0] == EVALS_PATH:
            self.reply(OK, list_body())
            return
        self.reply(OK, eval_body("fetched"))

    def do_DELETE(self) -> None:
        self.owner.record("DELETE", self.path, self.headers_dict(), self.body_bytes())
        if self.refused():
            return
        self.reply(OK, DELETED_BODY)

    def refused(self) -> bool:
        if self.owner.missing:
            self.reply(NOT_FOUND, MISSING_BODY)
            return True
        if self.owner.failing:
            self.reply(BAD_REQUEST, BAD_BODY)
            return True
        return False

    def headers_dict(self) -> dict[str, str]:
        return dict(self.headers.items())

    def body_bytes(self) -> bytes:
        declared = self.headers.get("Content-Length")
        if declared is None:
            return b""
        return self.rfile.read(int(declared))

    def reply(self, status: int, body: dict[str, JsonValue]) -> None:
        payload = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("content-type", JSON_TYPE)
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def eval_body(name: str) -> dict[str, JsonValue]:
    body: dict[str, JsonValue] = {
        "id": EVAL_ID,
        "object": "eval",
        "created_at": 1,
        "updated_at": 2,
        "name": name,
        "data_source_config": {"type": "custom", "item_schema": {}},
        "testing_criteria": [],
        "metadata": None,
    }
    return body


def list_body() -> dict[str, JsonValue]:
    body: dict[str, JsonValue] = {
        "object": "list",
        "data": [eval_body("listed")],
        "first_id": EVAL_ID,
        "last_id": EVAL_ID,
        "has_more": False,
    }
    return body


def parameter_rows(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        default: DefaultView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(default.default)))
    return rows


def dumped(model: BaseModel) -> dict[str, JsonValue]:
    values: dict[str, JsonValue] = model.model_dump(mode="json")
    return values


def header_of(call: Recorded, name: str) -> str | None:
    lowered = {key.lower(): value for key, value in call["headers"].items()}
    return lowered.get(name.lower())


def body_of(call: Recorded) -> dict[str, JsonValue]:
    values: dict[str, JsonValue] = json.loads(call["body"])
    return values


def made_answer(result: object) -> BaseModel:
    match result:
        case BaseModel():
            return result
        case Awaitable():
            raise AssertionError("the call answered with a coroutine")
        case object():
            raise AssertionError("the call did not answer with an eval")


PAIRS: Final[tuple[tuple[Callable[..., object], Callable[..., object]], ...]] = (
    (create_eval, LITELLM.create_eval),
    (acreate_eval, LITELLM.acreate_eval),
    (list_evals, LITELLM.list_evals),
    (alist_evals, LITELLM.alist_evals),
    (get_eval, LITELLM.get_eval),
    (aget_eval, LITELLM.aget_eval),
    (update_eval, LITELLM.update_eval),
    (aupdate_eval, LITELLM.aupdate_eval),
    (delete_eval, LITELLM.delete_eval),
    (adelete_eval, LITELLM.adelete_eval),
    (cancel_eval, LITELLM.cancel_eval),
    (acancel_eval, LITELLM.acancel_eval),
)


def test_signature_parameters_match_litellm() -> None:
    for ours, theirs in PAIRS:
        assert parameter_rows(inspect.signature(ours)) == parameter_rows(
            inspect.signature(theirs)
        )


def test_signature_all_parameters_are_accepted() -> None:
    with EvalsServer() as server:
        created = create_eval(
            DATA_SOURCE_CONFIG, TESTING_CRITERIA, NAME, METADATA, HEADERS, QUERY,
            EXTRA_BODY, TIMEOUT, OPENAI, api_base=server.url, api_key=GOOD_KEY,
        )
        listed = list_evals(
            5, "eval_a", "eval_b", "asc", "updated_at", HEADERS, QUERY, TIMEOUT, OPENAI,
            api_base=server.url, api_key=GOOD_KEY,
        )
        fetched = get_eval(
            EVAL_ID, HEADERS, QUERY, TIMEOUT, OPENAI, api_base=server.url, api_key=GOOD_KEY
        )
        updated = update_eval(
            EVAL_ID, NAME, METADATA, HEADERS, QUERY, EXTRA_BODY, TIMEOUT, OPENAI,
            api_base=server.url, api_key=GOOD_KEY,
        )
        deleted = delete_eval(
            EVAL_ID, HEADERS, QUERY, TIMEOUT, OPENAI, api_base=server.url, api_key=GOOD_KEY
        )
        cancelled = cancel_eval(
            EVAL_ID, HEADERS, QUERY, TIMEOUT, OPENAI, api_base=server.url, api_key=GOOD_KEY
        )
        reference_created = LITELLM.create_eval(
            data_source_config=DATA_SOURCE_CONFIG, testing_criteria=TESTING_CRITERIA,
            name=NAME, metadata=METADATA, extra_headers=HEADERS, extra_query=QUERY,
            extra_body=EXTRA_BODY, timeout=TIMEOUT, custom_llm_provider=OPENAI,
            api_base=server.url, api_key=GOOD_KEY,
        )
        reference_listed = LITELLM.list_evals(
            limit=5, after="eval_a", before="eval_b", order="asc", order_by="updated_at",
            extra_headers=HEADERS, extra_query=QUERY, timeout=TIMEOUT,
            custom_llm_provider=OPENAI, api_base=server.url, api_key=GOOD_KEY,
        )
        reference_fetched = LITELLM.get_eval(
            eval_id=EVAL_ID, extra_headers=HEADERS, extra_query=QUERY, timeout=TIMEOUT,
            custom_llm_provider=OPENAI, api_base=server.url, api_key=GOOD_KEY,
        )
        reference_updated = LITELLM.update_eval(
            eval_id=EVAL_ID, name=NAME, metadata=METADATA, extra_headers=HEADERS,
            extra_query=QUERY, extra_body=EXTRA_BODY, timeout=TIMEOUT,
            custom_llm_provider=OPENAI, api_base=server.url, api_key=GOOD_KEY,
        )
        reference_deleted = LITELLM.delete_eval(
            eval_id=EVAL_ID, extra_headers=HEADERS, extra_query=QUERY, timeout=TIMEOUT,
            custom_llm_provider=OPENAI, api_base=server.url, api_key=GOOD_KEY,
        )
        reference_cancelled = LITELLM.cancel_eval(
            eval_id=EVAL_ID, extra_headers=HEADERS, extra_query=QUERY, timeout=TIMEOUT,
            custom_llm_provider=OPENAI, api_base=server.url, api_key=GOOD_KEY,
        )
    assert dumped(made_answer(created)) == dumped(reference_created)
    assert dumped(made_answer(listed)) == dumped(reference_listed)
    assert dumped(made_answer(fetched)) == dumped(reference_fetched)
    assert dumped(made_answer(updated)) == dumped(reference_updated)
    assert dumped(made_answer(deleted)) == dumped(reference_deleted)
    assert dumped(made_answer(cancelled)) == dumped(reference_cancelled)
    ours_calls = server.calls[0::2]
    their_calls = server.calls[1::2]
    assert [(call["method"], call["path"]) for call in ours_calls] == [
        (call["method"], call["path"]) for call in their_calls
    ]


def test_signature_unknown_keyword() -> None:
    with EvalsServer() as server:
        ours = create_eval(
            DATA_SOURCE_CONFIG, TESTING_CRITERIA, api_base=server.url, api_key=GOOD_KEY,
            unknown=1,
        )
        reference = LITELLM.create_eval(
            data_source_config=DATA_SOURCE_CONFIG, testing_criteria=TESTING_CRITERIA,
            api_base=server.url, api_key=GOOD_KEY, unknown=1,
        )
    assert dumped(made_answer(ours)) == dumped(reference)
    assert body_of(server.calls[0]) == body_of(server.calls[1])


def test_signature_refuses_a_duplicated_keyword() -> None:
    with pytest.raises(TypeError, match="multiple values for keyword argument 'name'"):
        create_eval(DATA_SOURCE_CONFIG, TESTING_CRITERIA, name="first", **DUPLICATE)
    with pytest.raises(TypeError, match="multiple values for keyword argument 'name'"):
        LITELLM.create_eval(
            data_source_config=DATA_SOURCE_CONFIG, testing_criteria=TESTING_CRITERIA,
            name="first", **DUPLICATE,
        )


def test_calls_create() -> None:
    with EvalsServer() as server:
        ours = create_eval(
            DATA_SOURCE_CONFIG, TESTING_CRITERIA, name=NAME, metadata=METADATA,
            extra_headers=HEADERS, extra_query=QUERY, extra_body=EXTRA_BODY,
            api_base=server.url, api_key=GOOD_KEY,
        )
        reference = LITELLM.create_eval(
            data_source_config=DATA_SOURCE_CONFIG, testing_criteria=TESTING_CRITERIA,
            name=NAME, metadata=METADATA, extra_headers=HEADERS, extra_query=QUERY,
            extra_body=EXTRA_BODY, api_base=server.url, api_key=GOOD_KEY,
        )
    our_call = server.calls[-2]
    their_call = server.calls[-1]
    assert dumped(made_answer(ours)) == dumped(reference)
    assert dumped(made_answer(ours))["name"] == "created"
    assert our_call["method"] == their_call["method"] == "POST"
    assert our_call["path"] == their_call["path"] == EVALS_PATH
    assert body_of(our_call) == body_of(their_call)
    assert body_of(our_call) == {
        "data_source_config": DATA_SOURCE_CONFIG,
        "testing_criteria": TESTING_CRITERIA,
        "name": NAME,
        "note": "n",
    }
    assert header_of(our_call, CUSTOM_HEADER) == CUSTOM_VALUE
    assert header_of(our_call, "authorization") == f"Bearer {GOOD_KEY}"
    assert header_of(our_call, "content-type") == header_of(their_call, "content-type")
    assert header_of(our_call, "content-type") == JSON_TYPE


def test_calls_list() -> None:
    with EvalsServer() as server:
        ours = list_evals(
            limit=5, after="eval_a", before="eval_b", order="asc", order_by="updated_at",
            extra_headers=HEADERS, extra_query=QUERY, api_base=server.url, api_key=GOOD_KEY,
        )
        reference = LITELLM.list_evals(
            limit=5, after="eval_a", before="eval_b", order="asc", order_by="updated_at",
            extra_headers=HEADERS, extra_query=QUERY, api_base=server.url, api_key=GOOD_KEY,
        )
    our_call = server.calls[-2]
    their_call = server.calls[-1]
    assert dumped(made_answer(ours)) == dumped(reference)
    assert our_call["path"] == their_call["path"]
    expected = f"{EVALS_PATH}?limit=9&after=eval_x&before=eval_b&order=asc&order_by=updated_at"
    assert our_call["path"] == expected
    assert header_of(our_call, CUSTOM_HEADER) == CUSTOM_VALUE


def test_calls_list_sends_only_the_parameters_it_was_given() -> None:
    with EvalsServer() as server:
        ours = list_evals(limit=0, api_base=server.url, api_key=GOOD_KEY)
        reference = LITELLM.list_evals(limit=0, api_base=server.url, api_key=GOOD_KEY)
    assert dumped(made_answer(ours)) == dumped(reference)
    assert server.calls[-2]["path"] == server.calls[-1]["path"] == EVALS_PATH


def test_calls_get() -> None:
    with EvalsServer() as server:
        ours = get_eval(
            EVAL_ID, extra_headers=HEADERS, extra_query=QUERY, api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference = LITELLM.get_eval(
            eval_id=EVAL_ID, extra_headers=HEADERS, extra_query=QUERY, api_base=server.url,
            api_key=GOOD_KEY,
        )
    our_call = server.calls[-2]
    their_call = server.calls[-1]
    assert dumped(made_answer(ours)) == dumped(reference)
    assert dumped(made_answer(ours))["name"] == "fetched"
    assert our_call["method"] == their_call["method"] == "GET"
    assert our_call["path"] == their_call["path"] == f"{EVALS_PATH}/{EVAL_ID}"
    assert header_of(our_call, CUSTOM_HEADER) == CUSTOM_VALUE


def test_calls_update_filters_the_internal_metadata() -> None:
    with EvalsServer() as server:
        ours = update_eval(
            EVAL_ID, name=NAME, metadata=UPDATE_METADATA, extra_body=EXTRA_BODY,
            api_base=server.url, api_key=GOOD_KEY,
        )
        internal = update_eval(
            EVAL_ID, metadata=INTERNAL_METADATA, api_base=server.url, api_key=GOOD_KEY
        )
        replaced = update_eval(
            EVAL_ID, metadata=INTERNAL_METADATA, extra_body={"metadata": {"headers": "sent"}},
            api_base=server.url, api_key=GOOD_KEY,
        )
        reference = LITELLM.update_eval(
            eval_id=EVAL_ID, name=NAME, metadata=UPDATE_METADATA, extra_body=EXTRA_BODY,
            api_base=server.url, api_key=GOOD_KEY,
        )
        reference_internal = LITELLM.update_eval(
            eval_id=EVAL_ID, metadata=INTERNAL_METADATA, api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_replaced = LITELLM.update_eval(
            eval_id=EVAL_ID, metadata=INTERNAL_METADATA,
            extra_body={"metadata": {"headers": "sent"}}, api_base=server.url,
            api_key=GOOD_KEY,
        )
    assert dumped(made_answer(ours)) == dumped(reference)
    assert dumped(made_answer(internal)) == dumped(reference_internal)
    assert dumped(made_answer(replaced)) == dumped(reference_replaced)
    our_calls = server.calls[0::2]
    their_calls = server.calls[1::2]
    assert [body_of(call) for call in our_calls] == [body_of(call) for call in their_calls]
    assert body_of(our_calls[0]) == {"name": NAME, "metadata": {"team": "evals"}, "note": "n"}
    assert body_of(our_calls[1]) == {}
    assert body_of(our_calls[2]) == {"metadata": {"headers": "sent"}}


def test_calls_delete() -> None:
    with EvalsServer() as server:
        ours = delete_eval(EVAL_ID, extra_headers=HEADERS, api_base=server.url, api_key=GOOD_KEY)
        reference = LITELLM.delete_eval(
            eval_id=EVAL_ID, extra_headers=HEADERS, api_base=server.url, api_key=GOOD_KEY
        )
    our_call = server.calls[-2]
    their_call = server.calls[-1]
    assert dumped(made_answer(ours)) == dumped(reference)
    assert our_call["method"] == their_call["method"] == "DELETE"
    assert our_call["path"] == their_call["path"] == f"{EVALS_PATH}/{EVAL_ID}"
    assert our_call["body"] == their_call["body"] == b""
    assert header_of(our_call, CUSTOM_HEADER) == CUSTOM_VALUE


def test_calls_cancel() -> None:
    with EvalsServer() as server:
        ours = cancel_eval(
            EVAL_ID, extra_headers=HEADERS, api_base=server.url, api_key=GOOD_KEY
        )
        reference = LITELLM.cancel_eval(
            eval_id=EVAL_ID, extra_headers=HEADERS, api_base=server.url, api_key=GOOD_KEY
        )
    our_call = server.calls[-2]
    their_call = server.calls[-1]
    assert dumped(made_answer(ours)) == dumped(reference)
    assert our_call["method"] == their_call["method"] == "POST"
    assert our_call["path"] == their_call["path"] == f"{EVALS_PATH}/{EVAL_ID}{CANCEL_SUFFIX}"
    assert body_of(our_call) == body_of(their_call) == {}
    assert header_of(our_call, CUSTOM_HEADER) == CUSTOM_VALUE


def test_calls_async_all_operations() -> None:
    with EvalsServer() as server:
        created = asyncio.run(
            acreate_eval(
                DATA_SOURCE_CONFIG, TESTING_CRITERIA, name=NAME, api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        listed = asyncio.run(alist_evals(limit=5, api_base=server.url, api_key=GOOD_KEY))
        fetched = asyncio.run(aget_eval(EVAL_ID, api_base=server.url, api_key=GOOD_KEY))
        updated = asyncio.run(
            aupdate_eval(EVAL_ID, name=NAME, api_base=server.url, api_key=GOOD_KEY)
        )
        deleted = asyncio.run(adelete_eval(EVAL_ID, api_base=server.url, api_key=GOOD_KEY))
        cancelled = asyncio.run(acancel_eval(EVAL_ID, api_base=server.url, api_key=GOOD_KEY))
        reference_created = asyncio.run(
            LITELLM_ASYNC.acreate_eval(
                data_source_config=DATA_SOURCE_CONFIG, testing_criteria=TESTING_CRITERIA,
                name=NAME, api_base=server.url, api_key=GOOD_KEY,
            )
        )
        reference_listed = asyncio.run(
            LITELLM_ASYNC.alist_evals(limit=5, api_base=server.url, api_key=GOOD_KEY)
        )
        reference_fetched = asyncio.run(
            LITELLM_ASYNC.aget_eval(eval_id=EVAL_ID, api_base=server.url, api_key=GOOD_KEY)
        )
        reference_updated = asyncio.run(
            LITELLM_ASYNC.aupdate_eval(
                eval_id=EVAL_ID, name=NAME, api_base=server.url, api_key=GOOD_KEY
            )
        )
        reference_deleted = asyncio.run(
            LITELLM_ASYNC.adelete_eval(eval_id=EVAL_ID, api_base=server.url, api_key=GOOD_KEY)
        )
        reference_cancelled = asyncio.run(
            LITELLM_ASYNC.acancel_eval(eval_id=EVAL_ID, api_base=server.url, api_key=GOOD_KEY)
        )
    assert dumped(created) == dumped(reference_created)
    assert dumped(listed) == dumped(reference_listed)
    assert dumped(fetched) == dumped(reference_fetched)
    assert dumped(updated) == dumped(reference_updated)
    assert dumped(deleted) == dumped(reference_deleted)
    assert dumped(cancelled) == dumped(reference_cancelled)
    seen = [(call["method"], call["path"]) for call in server.calls]
    assert seen == [*OPERATIONS, *OPERATIONS]


def test_calls_ignore_the_litellm_logging_object() -> None:
    with EvalsServer() as server:
        created = create_eval(
            DATA_SOURCE_CONFIG, TESTING_CRITERIA, name=NAME, api_base=server.url,
            api_key=GOOD_KEY, litellm_logging_obj=object(),
        )
    assert dumped(made_answer(created))["name"] == "created"
    assert body_of(server.calls[0]) == {
        "data_source_config": DATA_SOURCE_CONFIG,
        "testing_criteria": TESTING_CRITERIA,
        "name": NAME,
    }


def test_error_provider_rejection() -> None:
    with EvalsServer(failing=True) as server:
        with pytest.raises(BadRequestError) as ours:
            create_eval(
                DATA_SOURCE_CONFIG, TESTING_CRITERIA, api_base=server.url, api_key=GOOD_KEY
            )
        with pytest.raises(LITELLM_ERRORS.BadRequestError) as theirs:
            LITELLM.create_eval(
                data_source_config=DATA_SOURCE_CONFIG, testing_criteria=TESTING_CRITERIA,
                api_base=server.url, api_key=GOOD_KEY,
            )
    assert ours.value.status_code == theirs.value.status_code == BAD_REQUEST
    assert BAD_MESSAGE in str(theirs.value)
    assert "openai: the provider rejected the request body" in str(ours.value)
    cause = ours.value.__cause__
    assert cause is not None
    assert '"BadRequestError"' in str(cause)
    assert f'"status_code":{BAD_REQUEST}' in str(cause)


def test_error_not_found() -> None:
    with EvalsServer(missing=True) as server:
        with pytest.raises(NotFoundError) as fetched:
            get_eval(EVAL_ID, api_base=server.url, api_key=GOOD_KEY)
        with pytest.raises(LITELLM_ERRORS.NotFoundError) as reference_fetched:
            LITELLM.get_eval(eval_id=EVAL_ID, api_base=server.url, api_key=GOOD_KEY)
        with pytest.raises(NotFoundError) as deleted:
            delete_eval(EVAL_ID, api_base=server.url, api_key=GOOD_KEY)
        with pytest.raises(LITELLM_ERRORS.NotFoundError) as reference_deleted:
            LITELLM.delete_eval(eval_id=EVAL_ID, api_base=server.url, api_key=GOOD_KEY)
        with pytest.raises(NotFoundError) as cancelled:
            cancel_eval(EVAL_ID, api_base=server.url, api_key=GOOD_KEY)
        with pytest.raises(LITELLM_ERRORS.NotFoundError) as reference_cancelled:
            LITELLM.cancel_eval(eval_id=EVAL_ID, api_base=server.url, api_key=GOOD_KEY)
    for ours, theirs in (
        (fetched, reference_fetched),
        (deleted, reference_deleted),
        (cancelled, reference_cancelled),
    ):
        assert ours.value.status_code == theirs.value.status_code == NOT_FOUND
        assert MISSING_MESSAGE in str(theirs.value)
        assert "openai: the provider could not find the requested resource" in str(ours.value)


def test_error_unsupported_providers() -> None:
    with pytest.raises(APIConnectionError) as created:
        create_eval(
            DATA_SOURCE_CONFIG, TESTING_CRITERIA, custom_llm_provider=ANTHROPIC,
            api_key=GOOD_KEY,
        )
    with pytest.raises(APIConnectionError) as listed:
        list_evals(custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY)
    with pytest.raises(APIConnectionError) as fetched:
        get_eval(EVAL_ID, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY)
    with pytest.raises(APIConnectionError) as updated:
        update_eval(EVAL_ID, name=NAME, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY)
    with pytest.raises(LITELLM_ERRORS.APIConnectionError) as reference_created:
        LITELLM.create_eval(
            data_source_config=DATA_SOURCE_CONFIG, testing_criteria=TESTING_CRITERIA,
            custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY,
        )
    with pytest.raises(LITELLM_ERRORS.APIConnectionError) as reference_listed:
        LITELLM.list_evals(custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY)
    with pytest.raises(LITELLM_ERRORS.APIConnectionError) as reference_fetched:
        LITELLM.get_eval(
            eval_id=EVAL_ID, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY
        )
    with pytest.raises(LITELLM_ERRORS.APIConnectionError) as reference_updated:
        LITELLM.update_eval(
            eval_id=EVAL_ID, name=NAME, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY
        )
    for ours, theirs, action in (
        (created, reference_created, "CREATE eval"),
        (listed, reference_listed, "LIST evals"),
        (fetched, reference_fetched, "GET eval"),
        (updated, reference_updated, "UPDATE eval"),
    ):
        assert ours.value.status_code == theirs.value.status_code == SERVER_ERROR
        assert f"{action} is not supported for {ANTHROPIC}" in str(ours.value)
        assert f"{action} is not supported for {ANTHROPIC}" in str(theirs.value)


def test_unsupported_lifecycle_provider_is_a_400() -> None:
    with pytest.raises(BadRequestError) as deleted:
        delete_eval(EVAL_ID, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY)
    with pytest.raises(BadRequestError) as cancelled:
        cancel_eval(EVAL_ID, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY)
    with pytest.raises(LITELLM_ERRORS.APIConnectionError) as reference_deleted:
        LITELLM.delete_eval(
            eval_id=EVAL_ID, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY
        )
    with pytest.raises(LITELLM_ERRORS.APIConnectionError) as reference_cancelled:
        LITELLM.cancel_eval(
            eval_id=EVAL_ID, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY
        )
    assert deleted.value.status_code == cancelled.value.status_code == BAD_REQUEST
    assert "DELETE eval is not supported for anthropic" in str(deleted.value)
    assert "CANCEL eval is not supported for anthropic" in str(cancelled.value)
    assert (
        reference_deleted.value.status_code
        == reference_cancelled.value.status_code
        == SERVER_ERROR
    )
    assert "DELETE eval is not supported for anthropic" in str(reference_deleted.value)
    assert "CANCEL eval is not supported for anthropic" in str(reference_cancelled.value)


def test_unsupported_object() -> None:
    with pytest.raises(UnsupportedCallArgument) as client_caught:
        create_eval(DATA_SOURCE_CONFIG, TESTING_CRITERIA, client=object())
    assert client_caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as session_caught:
        asyncio.run(aget_eval(EVAL_ID, shared_session=object()))
    assert session_caught.value.argument == "shared_session"
