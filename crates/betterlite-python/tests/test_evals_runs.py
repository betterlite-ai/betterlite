import asyncio
import importlib
import inspect
import json
import threading
from collections.abc import Awaitable
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, Protocol, Self

import httpx
import pytest
from pydantic import BaseModel, JsonValue
from test_evals_calls import (
    LITELLM,
    LITELLM_ASYNC,
    LITELLM_ERRORS,
    Recorded,
    awaited_answer,
    body_of,
    dumped,
    header_of,
    made_answer,
    parameter_rows,
    query_of,
    request_shape,
)
from typing_extensions import override

import betterlite
import betterlite.evals
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite.evals import main as evals_main
from betterlite.evals._runs import (
    acancel_run,
    acreate_run,
    adelete_run,
    aget_run,
    alist_runs,
    cancel_run,
    create_run,
    delete_run,
    get_run,
    list_runs,
)
from betterlite.exceptions import APIConnectionError, BadRequestError, NotFoundError

HOST: Final = "127.0.0.1"
OK: Final = 200
BAD_REQUEST: Final = 400
NOT_FOUND: Final = 404
SERVER_ERROR: Final = 500
GOOD_KEY: Final = "sk-test"
CLOSED_BASE: Final = "http://127.0.0.1:9"
EVAL_ID: Final = "eval_1"
RUN_ID: Final = "run_1"
RUNS_PATH: Final = f"/v1/evals/{EVAL_ID}/runs"
RUN_PATH: Final = f"{RUNS_PATH}/{RUN_ID}"
CANCEL_SUFFIX: Final = "/cancel"
CUSTOM_HEADER: Final = "x-eval-run-test"
CUSTOM_VALUE: Final = "on"
TIMEOUT: Final = 30.0
STRUCTURED_TIMEOUT: Final = httpx.Timeout(connect=1.0, read=60.0, write=1.0, pool=1.0)
ANTHROPIC: Final = "anthropic"
OPENAI: Final = "openai"
JSON_TYPE: Final = "application/json"
BAD_MESSAGE: Final = "the run request is invalid"
MISSING_MESSAGE: Final = "run not found"
NAME: Final = "probe"
CREDENTIAL_NAME: Final = "stored-openai"

DATA_SOURCE: Final[dict[str, JsonValue]] = {
    "type": "jsonl",
    "source": {"type": "file_id", "id": "file-1"},
}
METADATA: Final[dict[str, JsonValue]] = {"team": "evals"}
EXTRA_BODY: Final[dict[str, JsonValue]] = {"metadata": {"note": "n"}}
HEADERS: Final[dict[str, str]] = {CUSTOM_HEADER: CUSTOM_VALUE}
QUERY: Final[dict[str, JsonValue]] = {"limit": 9, "after": "run_x", "unknown": "x"}

BAD_BODY: Final[dict[str, JsonValue]] = {
    "type": "error",
    "error": {"type": "invalid_request_error", "message": BAD_MESSAGE},
}
MISSING_BODY: Final[dict[str, JsonValue]] = {
    "type": "error",
    "error": {"type": "not_found_error", "message": MISSING_MESSAGE},
}
CANCELLED_BODY: Final[dict[str, JsonValue]] = {
    "id": RUN_ID,
    "object": "eval.run",
    "status": "cancelled",
}
DELETED_BODY: Final[dict[str, JsonValue]] = {
    "run_id": RUN_ID,
    "object": "eval.run.deleted",
    "deleted": True,
}

# the calls both packages make, in order: create, list, get, cancel, delete
OPERATIONS: Final[tuple[tuple[str, str], ...]] = (
    ("POST", RUNS_PATH),
    ("GET", f"{RUNS_PATH}?limit=5"),
    ("GET", RUN_PATH),
    ("POST", f"{RUN_PATH}{CANCEL_SUFFIX}"),
    ("DELETE", RUN_PATH),
)

RUN_NAMES: Final[frozenset[str]] = frozenset({
    "acancel_run", "acreate_run", "adelete_run", "aget_run", "alist_runs",
    "cancel_run", "create_run", "delete_run", "get_run", "list_runs",
})
EVAL_NAMES: Final[frozenset[str]] = frozenset({
    "acancel_eval", "acreate_eval", "adelete_eval", "aget_eval", "alist_evals", "aupdate_eval",
    "cancel_eval", "create_eval", "delete_eval", "get_eval", "list_evals", "update_eval",
})


class StoredCredential(BaseModel):
    credential_name: str
    credential_info: dict[str, JsonValue]
    credential_values: dict[str, JsonValue]


class CredentialsModule(Protocol):
    def __getattr__(self, name: str) -> list[StoredCredential]: ...


LITELLM_CREDENTIALS: Final[CredentialsModule] = importlib.import_module("litellm")


class RunsServer(ThreadingHTTPServer):
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
        start = threading.Thread(target=self.serve_forever, daemon=True)
        start.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}"

    def record(self, method: str, path: str, headers: dict[str, str], body: bytes) -> None:
        entry: Recorded = {"method": method, "path": path, "headers": headers, "body": body}
        self.calls.append(entry)

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


class _Handler(BaseHTTPRequestHandler):
    owner: ClassVar[RunsServer]

    def do_POST(self) -> None:
        self.owner.record("POST", self.path, self.headers_dict(), self.body_bytes())
        if self.refused():
            return
        cancelling = self.path.partition("?")[0].endswith(CANCEL_SUFFIX)
        self.reply(OK, CANCELLED_BODY if cancelling else run_body("created"))

    def do_GET(self) -> None:
        self.owner.record("GET", self.path, self.headers_dict(), self.body_bytes())
        if self.refused():
            return
        if self.path.partition("?")[0] != RUNS_PATH:
            self.reply(OK, run_body("fetched"))
            return
        listing: dict[str, JsonValue] = {
            "object": "list", "data": [run_body("listed")], "has_more": False,
        }
        self.reply(OK, listing)

    def do_DELETE(self) -> None:
        self.owner.record("DELETE", self.path, self.headers_dict(), self.body_bytes())
        if self.refused():
            return
        self.reply(OK, DELETED_BODY)

    def refused(self) -> bool:
        match (self.owner.missing, self.owner.failing):
            case (True, _):
                self.reply(NOT_FOUND, MISSING_BODY)
            case (False, True):
                self.reply(BAD_REQUEST, BAD_BODY)
            case (False, False):
                return False
        return True

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


def run_body(name: str) -> dict[str, JsonValue]:
    body: dict[str, JsonValue] = {
        "id": RUN_ID, "object": "eval.run", "created_at": 1, "status": "queued",
        "data_source": {"type": "jsonl"}, "eval_id": EVAL_ID, "name": name,
        "started_at": None, "completed_at": None, "model": None,
        "per_model_usage": None, "per_testing_criteria_results": None,
        "report_url": None, "result_counts": None, "shared_with_openai": None,
        "metadata": None, "error": None,
    }
    return body


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(create_run)) == parameter_rows(
        inspect.signature(LITELLM.create_run)
    )
    assert parameter_rows(inspect.signature(acreate_run)) == parameter_rows(
        inspect.signature(LITELLM.acreate_run)
    )
    assert parameter_rows(inspect.signature(list_runs)) == parameter_rows(
        inspect.signature(LITELLM.list_runs)
    )
    assert parameter_rows(inspect.signature(alist_runs)) == parameter_rows(
        inspect.signature(LITELLM.alist_runs)
    )
    assert parameter_rows(inspect.signature(get_run)) == parameter_rows(
        inspect.signature(LITELLM.get_run)
    )
    assert parameter_rows(inspect.signature(aget_run)) == parameter_rows(
        inspect.signature(LITELLM.aget_run)
    )
    assert parameter_rows(inspect.signature(cancel_run)) == parameter_rows(
        inspect.signature(LITELLM.cancel_run)
    )
    assert parameter_rows(inspect.signature(acancel_run)) == parameter_rows(
        inspect.signature(LITELLM.acancel_run)
    )
    assert parameter_rows(inspect.signature(delete_run)) == parameter_rows(
        inspect.signature(LITELLM.delete_run)
    )
    assert parameter_rows(inspect.signature(adelete_run)) == parameter_rows(
        inspect.signature(LITELLM.adelete_run)
    )


def test_signature_all_parameters() -> None:
    with RunsServer() as server:
        created = create_run(
            EVAL_ID, DATA_SOURCE, NAME, METADATA, HEADERS, QUERY, EXTRA_BODY, TIMEOUT, OPENAI,
            api_base=server.url, api_key=GOOD_KEY,
        )
        listed = list_runs(
            EVAL_ID, 5, "run_a", "run_b", "asc", HEADERS, QUERY, TIMEOUT, OPENAI,
            api_base=server.url, api_key=GOOD_KEY,
        )
        fetched = get_run(
            EVAL_ID, RUN_ID, HEADERS, QUERY, TIMEOUT, OPENAI,
            api_base=server.url, api_key=GOOD_KEY,
        )
        cancelled = cancel_run(
            EVAL_ID, RUN_ID, HEADERS, QUERY, TIMEOUT, OPENAI,
            api_base=server.url, api_key=GOOD_KEY,
        )
        deleted = delete_run(
            EVAL_ID, RUN_ID, HEADERS, QUERY, TIMEOUT, OPENAI,
            api_base=server.url, api_key=GOOD_KEY,
        )
        reference_created = LITELLM.create_run(
            eval_id=EVAL_ID, data_source=DATA_SOURCE, name=NAME, metadata=METADATA,
            extra_headers=HEADERS, extra_query=QUERY, extra_body=EXTRA_BODY, timeout=TIMEOUT,
            custom_llm_provider=OPENAI, api_base=server.url, api_key=GOOD_KEY,
        )
        reference_listed = LITELLM.list_runs(
            eval_id=EVAL_ID, limit=5, after="run_a", before="run_b", order="asc",
            extra_headers=HEADERS, extra_query=QUERY, timeout=TIMEOUT,
            custom_llm_provider=OPENAI, api_base=server.url, api_key=GOOD_KEY,
        )
        reference_fetched = LITELLM.get_run(
            eval_id=EVAL_ID, run_id=RUN_ID, extra_headers=HEADERS, extra_query=QUERY,
            timeout=TIMEOUT, custom_llm_provider=OPENAI, api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_cancelled = LITELLM.cancel_run(
            eval_id=EVAL_ID, run_id=RUN_ID, extra_headers=HEADERS, extra_query=QUERY,
            timeout=TIMEOUT, custom_llm_provider=OPENAI, api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_deleted = LITELLM.delete_run(
            eval_id=EVAL_ID, run_id=RUN_ID, extra_headers=HEADERS, extra_query=QUERY,
            timeout=TIMEOUT, custom_llm_provider=OPENAI, api_base=server.url,
            api_key=GOOD_KEY,
        )
    assert dumped(made_answer(created)) == dumped(reference_created)
    assert dumped(made_answer(listed)) == dumped(reference_listed)
    assert dumped(made_answer(fetched)) == dumped(reference_fetched)
    assert dumped(made_answer(cancelled)) == dumped(reference_cancelled)
    assert dumped(made_answer(deleted)) == dumped(reference_deleted)
    ours_calls = server.calls[:5]
    their_calls = server.calls[5:]
    assert [request_shape(call) for call in ours_calls] == [
        request_shape(call) for call in their_calls
    ]


def test_signature_unknown_keyword() -> None:
    with RunsServer() as server:
        answers: list[BaseModel | Awaitable[BaseModel]] = [
            create_run(
                EVAL_ID, DATA_SOURCE, api_base=server.url, api_key=GOOD_KEY, unknown=1,
            ),
            LITELLM.create_run(
                eval_id=EVAL_ID, data_source=DATA_SOURCE, api_base=server.url,
                api_key=GOOD_KEY, unknown=1,
            ),
        ]
        answers_dumped = [dumped(made_answer(item)) for item in answers]
        bodies = [body_of(call) for call in server.calls]
    assert answers_dumped[0] == answers_dumped[1]
    assert bodies[0] == bodies[1]


def test_calls_create() -> None:
    with RunsServer() as server:
        ours = create_run(
            EVAL_ID, DATA_SOURCE, name=NAME, metadata=METADATA, extra_headers=HEADERS,
            extra_query=QUERY, extra_body=EXTRA_BODY, api_base=server.url, api_key=GOOD_KEY,
        )
        reference = LITELLM.create_run(
            eval_id=EVAL_ID, data_source=DATA_SOURCE, name=NAME, metadata=METADATA,
            extra_headers=HEADERS, extra_query=QUERY, extra_body=EXTRA_BODY,
            api_base=server.url, api_key=GOOD_KEY,
        )
    our_call = server.calls[-2]
    their_call = server.calls[-1]
    assert dumped(made_answer(ours)) == dumped(reference)
    assert dumped(made_answer(ours))["name"] == "created"
    assert our_call["method"] == their_call["method"] == "POST"
    assert our_call["path"] == their_call["path"] == RUNS_PATH
    assert body_of(our_call) == body_of(their_call)
    assert body_of(our_call) == {
        "data_source": DATA_SOURCE,
        "name": NAME,
        "metadata": {"note": "n"},
    }
    assert header_of(our_call, CUSTOM_HEADER) == CUSTOM_VALUE
    assert header_of(our_call, "authorization") == f"Bearer {GOOD_KEY}"
    assert header_of(our_call, "content-type") == header_of(their_call, "content-type")
    assert header_of(our_call, "content-type") == JSON_TYPE


# LiteLLM accepts create_run's metadata argument and never puts it on the request
def test_calls_drop_metadata() -> None:
    with RunsServer() as server:
        named = create_run(
            EVAL_ID, DATA_SOURCE, metadata=METADATA, api_base=server.url, api_key=GOOD_KEY
        )
        bodied = create_run(
            EVAL_ID, DATA_SOURCE, metadata=METADATA, extra_body=EXTRA_BODY,
            api_base=server.url, api_key=GOOD_KEY,
        )
        reference_named = LITELLM.create_run(
            eval_id=EVAL_ID, data_source=DATA_SOURCE, metadata=METADATA,
            api_base=server.url, api_key=GOOD_KEY,
        )
        reference_bodied = LITELLM.create_run(
            eval_id=EVAL_ID, data_source=DATA_SOURCE, metadata=METADATA,
            extra_body=EXTRA_BODY, api_base=server.url, api_key=GOOD_KEY,
        )
        bodies = [body_of(call) for call in server.calls]
    assert dumped(made_answer(named)) == dumped(reference_named)
    assert dumped(made_answer(bodied)) == dumped(reference_bodied)
    assert bodies[0] == bodies[2] == {"data_source": DATA_SOURCE}
    assert bodies[1] == bodies[3] == {"data_source": DATA_SOURCE, "metadata": {"note": "n"}}


def test_calls_list() -> None:
    with RunsServer() as server:
        reference = LITELLM.list_runs(
            eval_id=EVAL_ID, limit=5, after="run_a", before="run_b", order="asc",
            extra_headers=HEADERS, extra_query=QUERY, api_base=server.url, api_key=GOOD_KEY,
        )
        ours = list_runs(
            eval_id=EVAL_ID, limit=5, after="run_a", before="run_b", order="asc",
            extra_headers=HEADERS, extra_query=QUERY, api_base=server.url, api_key=GOOD_KEY,
        )
    our_call = server.calls[-1]
    their_call = server.calls[-2]
    assert dumped(made_answer(ours)) == dumped(reference)
    assert our_call["path"].startswith(f"{RUNS_PATH}?")
    assert query_of(our_call) == {
        "limit": "9",
        "after": "run_x",
        "before": "run_b",
        "order": "asc",
    }
    assert request_shape(our_call) == request_shape(their_call)
    assert header_of(our_call, CUSTOM_HEADER) == CUSTOM_VALUE


def test_calls_list_sends_given() -> None:
    with RunsServer() as server:
        ours = list_runs(
            eval_id=EVAL_ID, limit=0, after="", before="", order=None,
            api_base=server.url, api_key=GOOD_KEY,
        )
        reference = LITELLM.list_runs(
            eval_id=EVAL_ID, limit=0, after="", before="", order=None,
            api_base=server.url, api_key=GOOD_KEY,
        )
        paths = [call["path"] for call in server.calls]
    assert dumped(made_answer(ours)) == dumped(reference)
    assert paths == [RUNS_PATH, RUNS_PATH]


def test_calls_get() -> None:
    with RunsServer() as server:
        bare = get_run(EVAL_ID, RUN_ID, api_base=server.url, api_key=GOOD_KEY)
        ours = get_run(
            EVAL_ID, RUN_ID, extra_headers=HEADERS, extra_query=QUERY,
            api_base=server.url, api_key=GOOD_KEY,
        )
        reference = LITELLM.get_run(
            eval_id=EVAL_ID, run_id=RUN_ID, extra_headers=HEADERS, extra_query=QUERY,
            api_base=server.url, api_key=GOOD_KEY,
        )
        seen = [(call["method"], call["path"]) for call in server.calls]
    fetched = made_answer(ours)
    assert seen == [("GET", RUN_PATH), ("GET", RUN_PATH), ("GET", RUN_PATH)]
    assert dumped(made_answer(bare)) == dumped(fetched) == dumped(reference)
    assert dumped(fetched)["name"] == "fetched"
    assert header_of(server.calls[0], CUSTOM_HEADER) is None
    assert header_of(server.calls[1], CUSTOM_HEADER) == CUSTOM_VALUE


def test_calls_cancel() -> None:
    with RunsServer() as server:
        made = cancel_run(
            EVAL_ID, RUN_ID, extra_headers=HEADERS, extra_query=QUERY,
            api_base=server.url, api_key=GOOD_KEY,
        )
        reference = LITELLM.cancel_run(
            eval_id=EVAL_ID, run_id=RUN_ID, extra_headers=HEADERS, extra_query=QUERY,
            api_base=server.url, api_key=GOOD_KEY,
        )
        seen = [(call["method"], call["path"], body_of(call)) for call in server.calls]
    cancelled = made_answer(made)
    assert seen == [("POST", f"{RUN_PATH}{CANCEL_SUFFIX}", {})] * 2
    assert dumped(cancelled) == dumped(reference)
    assert (dumped(cancelled)["status"], dumped(cancelled)["id"]) == ("cancelled", RUN_ID)
    assert header_of(server.calls[0], CUSTOM_HEADER) == CUSTOM_VALUE


def test_calls_delete() -> None:
    with RunsServer() as server:
        reference = LITELLM.delete_run(
            eval_id=EVAL_ID, run_id=RUN_ID, extra_headers=HEADERS, api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours = delete_run(
            EVAL_ID, RUN_ID, extra_headers=HEADERS, api_base=server.url, api_key=GOOD_KEY
        )
    our_call = server.calls[-1]
    their_call = server.calls[-2]
    assert our_call["body"] == their_call["body"] == b""
    assert dumped(made_answer(ours)) == dumped(reference)
    assert dumped(made_answer(ours))["deleted"] is True
    assert our_call["method"] == their_call["method"] == "DELETE"
    assert our_call["path"] == their_call["path"] == RUN_PATH
    assert header_of(our_call, CUSTOM_HEADER) == CUSTOM_VALUE


def test_calls_async_every_op() -> None:
    with RunsServer() as server:
        created = asyncio.run(
            acreate_run(
                EVAL_ID, DATA_SOURCE, name=NAME, extra_body=EXTRA_BODY,
                extra_headers=HEADERS, extra_query=QUERY, timeout=TIMEOUT,
                custom_llm_provider=OPENAI, api_base=server.url, api_key=GOOD_KEY,
            )
        )
        listed = asyncio.run(
            alist_runs(
                EVAL_ID, limit=5, extra_headers=HEADERS, extra_query=QUERY,
                timeout=TIMEOUT, custom_llm_provider=OPENAI, api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        fetched = asyncio.run(
            aget_run(
                EVAL_ID, RUN_ID, extra_headers=HEADERS, extra_query=QUERY,
                timeout=TIMEOUT, custom_llm_provider=OPENAI, api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        cancelled = asyncio.run(
            acancel_run(
                EVAL_ID, RUN_ID, extra_headers=HEADERS, extra_query=QUERY,
                timeout=TIMEOUT, custom_llm_provider=OPENAI, api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        deleted = asyncio.run(
            adelete_run(
                EVAL_ID, RUN_ID, extra_headers=HEADERS, extra_query=QUERY,
                timeout=TIMEOUT, custom_llm_provider=OPENAI, api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        reference_created = asyncio.run(
            LITELLM_ASYNC.acreate_run(
                eval_id=EVAL_ID, data_source=DATA_SOURCE, name=NAME, extra_body=EXTRA_BODY,
                extra_headers=HEADERS, extra_query=QUERY, timeout=TIMEOUT,
                custom_llm_provider=OPENAI, api_base=server.url, api_key=GOOD_KEY,
            )
        )
        reference_listed = asyncio.run(
            LITELLM_ASYNC.alist_runs(
                eval_id=EVAL_ID, limit=5, extra_headers=HEADERS, extra_query=QUERY,
                timeout=TIMEOUT, custom_llm_provider=OPENAI, api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        reference_fetched = asyncio.run(
            LITELLM_ASYNC.aget_run(
                eval_id=EVAL_ID, run_id=RUN_ID, extra_headers=HEADERS, extra_query=QUERY,
                timeout=TIMEOUT, custom_llm_provider=OPENAI, api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        reference_cancelled = asyncio.run(
            LITELLM_ASYNC.acancel_run(
                eval_id=EVAL_ID, run_id=RUN_ID, extra_headers=HEADERS, extra_query=QUERY,
                timeout=TIMEOUT, custom_llm_provider=OPENAI, api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        reference_deleted = asyncio.run(
            LITELLM_ASYNC.adelete_run(
                eval_id=EVAL_ID, run_id=RUN_ID, extra_headers=HEADERS, extra_query=QUERY,
                timeout=TIMEOUT, custom_llm_provider=OPENAI, api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
    assert dumped(created) == dumped(reference_created)
    assert dumped(listed) == dumped(reference_listed)
    assert dumped(fetched) == dumped(reference_fetched)
    assert dumped(cancelled) == dumped(reference_cancelled)
    assert dumped(deleted) == dumped(reference_deleted)
    ours_calls = server.calls[:5]
    their_calls = server.calls[5:]
    assert [request_shape(call) for call in ours_calls] == [
        request_shape(call) for call in their_calls
    ]
    assert [header_of(call, CUSTOM_HEADER) for call in server.calls] == [CUSTOM_VALUE] * 10
    assert query_of(ours_calls[1]) == {"limit": "9", "after": "run_x"}


def test_calls_flag_coroutines() -> None:
    with RunsServer() as server:
        created = create_run(
            EVAL_ID, DATA_SOURCE, name=NAME, extra_headers=HEADERS, extra_query=QUERY,
            timeout=TIMEOUT, custom_llm_provider=OPENAI, api_base=server.url,
            api_key=GOOD_KEY, acreate_run=True,
        )
        assert inspect.isawaitable(created)
        listed = list_runs(
            eval_id=EVAL_ID, limit=5, extra_headers=HEADERS, extra_query=QUERY,
            timeout=TIMEOUT, custom_llm_provider=OPENAI, api_base=server.url,
            api_key=GOOD_KEY, alist_runs=True,
        )
        assert inspect.isawaitable(listed)
        fetched = get_run(
            EVAL_ID, RUN_ID, extra_headers=HEADERS, extra_query=QUERY, timeout=TIMEOUT,
            custom_llm_provider=OPENAI, api_base=server.url, api_key=GOOD_KEY,
            aget_run=True,
        )
        assert inspect.isawaitable(fetched)
        cancelled = cancel_run(
            EVAL_ID, RUN_ID, extra_headers=HEADERS, extra_query=QUERY, timeout=TIMEOUT,
            custom_llm_provider=OPENAI, api_base=server.url, api_key=GOOD_KEY,
            acancel_run=True,
        )
        assert inspect.isawaitable(cancelled)
        deleted = delete_run(
            EVAL_ID, RUN_ID, extra_headers=HEADERS, extra_query=QUERY, timeout=TIMEOUT,
            custom_llm_provider=OPENAI, api_base=server.url, api_key=GOOD_KEY,
            adelete_run=True,
        )
        assert inspect.isawaitable(deleted)
        pending: tuple[BaseModel | Awaitable[BaseModel], ...] = (
            created, listed, fetched, cancelled, deleted,
        )
        answers = [asyncio.run(awaited_answer(item)) for item in pending]
        seen = [(call["method"], call["path"].partition("?")[0]) for call in server.calls]
    assert dumped(answers[0])["name"] == "created"
    assert dumped(answers[1])["object"] == "list"
    assert dumped(answers[2])["name"] == "fetched"
    assert dumped(answers[3])["status"] == "cancelled"
    assert dumped(answers[4])["deleted"] is True
    assert seen == [(method, path.partition("?")[0]) for method, path in OPERATIONS]
    assert [header_of(call, CUSTOM_HEADER) for call in server.calls] == [CUSTOM_VALUE] * 5
    assert query_of(server.calls[1]) == {"limit": "9", "after": "run_x"}


def test_calls_drop_logging() -> None:
    with RunsServer() as server:
        created = create_run(
            EVAL_ID, DATA_SOURCE, name=NAME, api_base=server.url, api_key=GOOD_KEY,
            litellm_logging_obj=object(),
        )
    assert dumped(made_answer(created))["name"] == "created"
    assert body_of(server.calls[0]) == {"data_source": DATA_SOURCE, "name": NAME}


# LiteLLM's wrapper fills the call from a matching litellm_credential_name before the body runs
def test_calls_credential_name() -> None:
    with RunsServer() as server:
        values: dict[str, JsonValue] = {"api_key": GOOD_KEY, "api_base": server.url}
        stored: dict[str, JsonValue] = {
            "credential_name": CREDENTIAL_NAME,
            "credential_values": values,
        }
        theirs = StoredCredential(
            credential_name=CREDENTIAL_NAME, credential_info={}, credential_values=values
        )
        betterlite.credential_list.append(stored)
        LITELLM_CREDENTIALS.credential_list.append(theirs)
        try:
            ours = create_run(EVAL_ID, DATA_SOURCE, litellm_credential_name=CREDENTIAL_NAME)
            reference = LITELLM.create_run(
                eval_id=EVAL_ID, data_source=DATA_SOURCE,
                litellm_credential_name=CREDENTIAL_NAME,
            )
        finally:
            betterlite.credential_list.remove(stored)
            LITELLM_CREDENTIALS.credential_list.remove(theirs)
        our_call = server.calls[0]
        their_call = server.calls[1]
    assert dumped(made_answer(ours)) == dumped(reference)
    assert our_call["path"] == their_call["path"] == RUNS_PATH
    assert header_of(our_call, "authorization") == f"Bearer {GOOD_KEY}"
    assert header_of(their_call, "authorization") == f"Bearer {GOOD_KEY}"


# a structured httpx.Timeout becomes one deadline; docs/deviations/B7886.yaml records the phase loss
def test_calls_structured_timeout() -> None:
    with RunsServer() as server:
        ours = create_run(
            EVAL_ID, DATA_SOURCE, timeout=STRUCTURED_TIMEOUT, api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference = LITELLM.create_run(
            eval_id=EVAL_ID, data_source=DATA_SOURCE, timeout=STRUCTURED_TIMEOUT,
            api_base=server.url, api_key=GOOD_KEY,
        )
        shapes = [request_shape(call) for call in server.calls]
    assert dumped(made_answer(ours)) == dumped(reference)
    assert shapes[0] == shapes[1]


# a call's ssl_verify never reaches the transport; docs/deviations/B7886.yaml records the gap
def test_calls_ignore_ssl_verify() -> None:
    with RunsServer() as server:
        ours = create_run(
            EVAL_ID, DATA_SOURCE, ssl_verify=False, api_base=server.url, api_key=GOOD_KEY
        )
        reference = LITELLM.create_run(
            eval_id=EVAL_ID, data_source=DATA_SOURCE, ssl_verify=False,
            api_base=server.url, api_key=GOOD_KEY,
        )
    assert dumped(made_answer(ours)) == dumped(reference)
    assert body_of(server.calls[0]) == body_of(server.calls[1])


# betterlite.evals carries the names of the eval calls and the run calls the task adds
def test_calls_export_every_name() -> None:
    names = RUN_NAMES | EVAL_NAMES
    assert len(betterlite.evals.__all__) == len(names)
    assert set(betterlite.evals.__all__) == names
    assert len(evals_main.__all__) == len(names)
    assert set(evals_main.__all__) == names


def test_error_provider_rejection() -> None:
    with RunsServer(failing=True) as server:
        with pytest.raises(BadRequestError) as ours:
            create_run(EVAL_ID, DATA_SOURCE, api_base=server.url, api_key=GOOD_KEY)
        with pytest.raises(LITELLM_ERRORS.BadRequestError) as theirs:
            LITELLM.create_run(
                eval_id=EVAL_ID, data_source=DATA_SOURCE, api_base=server.url,
                api_key=GOOD_KEY,
            )
    assert ours.value.status_code == theirs.value.status_code == BAD_REQUEST
    assert BAD_MESSAGE in str(theirs.value)
    assert BAD_MESSAGE in str(ours.value)
    cause = ours.value.__cause__
    assert cause is not None
    assert '"BadRequestError"' in str(cause)
    assert f'"status_code":{BAD_REQUEST}' in str(cause)


def test_error_not_found() -> None:
    with RunsServer(missing=True) as server:
        with pytest.raises(APIConnectionError) as fetched:
            get_run(EVAL_ID, RUN_ID, api_base=server.url, api_key=GOOD_KEY)
        with pytest.raises(LITELLM_ERRORS.APIConnectionError) as reference_fetched:
            LITELLM.get_run(
                eval_id=EVAL_ID, run_id=RUN_ID, api_base=server.url, api_key=GOOD_KEY
            )
        with pytest.raises(NotFoundError) as deleted:
            delete_run(EVAL_ID, RUN_ID, api_base=server.url, api_key=GOOD_KEY)
        with pytest.raises(LITELLM_ERRORS.NotFoundError) as reference_deleted:
            LITELLM.delete_run(
                eval_id=EVAL_ID, run_id=RUN_ID, api_base=server.url, api_key=GOOD_KEY
            )
        with pytest.raises(NotFoundError) as cancelled:
            cancel_run(EVAL_ID, RUN_ID, api_base=server.url, api_key=GOOD_KEY)
        with pytest.raises(LITELLM_ERRORS.NotFoundError) as reference_cancelled:
            LITELLM.cancel_run(
                eval_id=EVAL_ID, run_id=RUN_ID, api_base=server.url, api_key=GOOD_KEY
            )
    assert fetched.value.status_code == reference_fetched.value.status_code == SERVER_ERROR
    assert "the eval reply does not match its model" in str(fetched.value)
    for ours, theirs in ((deleted, reference_deleted), (cancelled, reference_cancelled)):
        assert ours.value.status_code == theirs.value.status_code == NOT_FOUND
        assert MISSING_MESSAGE in str(theirs.value)
        assert MISSING_MESSAGE in str(ours.value)


def test_error_unsupported() -> None:
    with pytest.raises(BadRequestError) as created:
        create_run(EVAL_ID, DATA_SOURCE, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY)
    with pytest.raises(BadRequestError) as listed:
        list_runs(EVAL_ID, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY)
    with pytest.raises(BadRequestError) as fetched:
        get_run(EVAL_ID, RUN_ID, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY)
    with pytest.raises(BadRequestError) as cancelled:
        cancel_run(EVAL_ID, RUN_ID, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY)
    with pytest.raises(BadRequestError) as deleted:
        delete_run(EVAL_ID, RUN_ID, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY)
    with pytest.raises(LITELLM_ERRORS.APIConnectionError) as reference_created:
        LITELLM.create_run(
            eval_id=EVAL_ID, data_source=DATA_SOURCE, custom_llm_provider=ANTHROPIC,
            api_key=GOOD_KEY,
        )
    with pytest.raises(LITELLM_ERRORS.APIConnectionError) as reference_listed:
        LITELLM.list_runs(eval_id=EVAL_ID, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY)
    with pytest.raises(LITELLM_ERRORS.APIConnectionError) as reference_fetched:
        LITELLM.get_run(
            eval_id=EVAL_ID, run_id=RUN_ID, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY
        )
    with pytest.raises(LITELLM_ERRORS.APIConnectionError) as reference_cancelled:
        LITELLM.cancel_run(
            eval_id=EVAL_ID, run_id=RUN_ID, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY
        )
    with pytest.raises(LITELLM_ERRORS.APIConnectionError) as reference_deleted:
        LITELLM.delete_run(
            eval_id=EVAL_ID, run_id=RUN_ID, custom_llm_provider=ANTHROPIC, api_key=GOOD_KEY
        )
    for ours, theirs, action in (
        (created, reference_created, "CREATE run"),
        (listed, reference_listed, "LIST runs"),
        (fetched, reference_fetched, "GET run"),
        (cancelled, reference_cancelled, "CANCEL run"),
        (deleted, reference_deleted, "DELETE run"),
    ):
        assert ours.value.status_code == BAD_REQUEST
        assert theirs.value.status_code == SERVER_ERROR
        assert f"{action} is not supported for {ANTHROPIC}" in str(ours.value)
        assert f"{action} is not supported for {ANTHROPIC}" in str(theirs.value)
    with RunsServer() as server:
        with pytest.raises(BadRequestError) as async_created:
            asyncio.run(
                acreate_run(
                    EVAL_ID, DATA_SOURCE, custom_llm_provider=ANTHROPIC,
                    api_base=server.url, api_key=GOOD_KEY,
                )
            )
    assert async_created.value.status_code == BAD_REQUEST
    assert f"CREATE run is not supported for {ANTHROPIC}" in str(async_created.value)


# the async twins forward timeout, so a value the native request refuses fails each call
def test_error_async_timeout() -> None:
    with pytest.raises(BadRequestError) as created:
        asyncio.run(
            acreate_run(
                EVAL_ID, DATA_SOURCE, timeout=-1.0, api_base=CLOSED_BASE, api_key=GOOD_KEY
            )
        )
    with pytest.raises(BadRequestError) as listed:
        asyncio.run(alist_runs(EVAL_ID, timeout=-1.0, api_base=CLOSED_BASE, api_key=GOOD_KEY))
    with pytest.raises(BadRequestError) as fetched:
        asyncio.run(
            aget_run(EVAL_ID, RUN_ID, timeout=-1.0, api_base=CLOSED_BASE, api_key=GOOD_KEY)
        )
    with pytest.raises(BadRequestError) as cancelled:
        asyncio.run(
            acancel_run(EVAL_ID, RUN_ID, timeout=-1.0, api_base=CLOSED_BASE, api_key=GOOD_KEY)
        )
    with pytest.raises(BadRequestError) as deleted:
        asyncio.run(
            adelete_run(EVAL_ID, RUN_ID, timeout=-1.0, api_base=CLOSED_BASE, api_key=GOOD_KEY)
        )
    for raised in (created, listed, fetched, cancelled, deleted):
        assert raised.value.status_code == BAD_REQUEST
        assert "timeout" in str(raised.value)


def test_unsupported_object() -> None:
    with pytest.raises(UnsupportedCallArgument) as client_caught:
        create_run(EVAL_ID, DATA_SOURCE, client=object())
    assert client_caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as session_caught:
        asyncio.run(aget_run(EVAL_ID, RUN_ID, shared_session=object()))
    assert session_caught.value.argument == "shared_session"
