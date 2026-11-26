import asyncio
import importlib
import inspect
import json
import threading
import time
from collections.abc import Awaitable, Iterator
from contextlib import closing, contextmanager, nullcontext
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, Protocol, SupportsIndex, TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

import betterlite
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._endpoints import CallValue
from betterlite._rerank import main as rerank_module
from betterlite._rerank.limits import (
    MAX_DEPTH,
    MAX_HEADERS,
    MAX_REQUEST_BYTES,
    STRING_CHUNK,
    RequestBudget,
    check_request_size,
)
from betterlite._rerank.main import MAX_DOCUMENTS, JsonValidator
from betterlite.exceptions import (
    APIConnectionError,
    AuthenticationError,
    BadRequestError,
    InternalServerError,
)
from betterlite.main import arerank, rerank
from betterlite.types.rerank import RerankResponse

HOST: Final = "127.0.0.1"
OK: Final = 200
AUTH_STATUS: Final = 401
BAD_REQUEST_STATUS: Final = 400
SERVER_ERROR_STATUS: Final = 500
JSON_TYPE: Final = "application/json"
GOOD_KEY: Final = "sk-test"
BAD_KEY: Final = "bad-key"
MODEL: Final = "rerank-english-v3.0"
PROVIDER: Final = "cohere"
QUERY: Final = "who wrote this"
CALL_ID: Final = "call-1"
MODEL_ID: Final = "deployment-1"
FIRST_INDEX: Final = 0
SECOND_INDEX: Final = 1
FIRST_SCORE: Final = 0.9
SECOND_SCORE: Final = 0.4
SEARCH_UNITS: Final = 1
SEARCH_COST: Final = 0.002
MAX_CHUNKS: Final = 10
WIRE_LIMIT: Final = 8 * 1024 * 1024
PROVIDER_DELAY: Final = 0.5
INFINITY_PROVIDER: Final = "infinity"
INFINITY_KEY: Final = "infinity-key-1"
MISSING_MODEL_TEXT: Final = "model parameter is required but was None"
UNSUPPORTED_TEXT: Final = "Unsupported provider: openai"
NOT_WIRED_TEXT: Final = "the together_ai rerank handler is not wired into betterlite"
AUTH_TEXT: Final = "invalid api key"

DOCUMENTS_TEXT: Final[list[str | dict[str, JsonValue]]] = [
    "first document",
    {"text": "second document"},
]
RANK_FIELDS: Final[list[str]] = ["text"]
# the provider lists the second document first; the SDK must keep that order
RESULTS: Final[list[JsonValue]] = [
    {"index": SECOND_INDEX, "relevance_score": SECOND_SCORE, "document": {"text": "second"}},
    {"index": FIRST_INDEX, "relevance_score": FIRST_SCORE, "document": {"text": "first"}},
]
RERANK_BODY: Final[dict[str, JsonValue]] = {
    "id": "rerank-1",
    "results": RESULTS,
    "meta": {"billed_units": {"search_units": SEARCH_UNITS}},
}
NO_DOCUMENT_BODY: Final[dict[str, JsonValue]] = {
    "id": "rerank-2",
    "results": [{"index": FIRST_INDEX, "relevance_score": FIRST_SCORE}],
    "meta": {"billed_units": {"search_units": SEARCH_UNITS}},
}
AUTH_BODY: Final[dict[str, JsonValue]] = {"message": AUTH_TEXT}

DUMP: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
RECORDS: Final[TypeAdapter[list[dict[str, JsonValue]]]] = TypeAdapter(
    list[dict[str, JsonValue]]
)
FLOAT: Final[TypeAdapter[float]] = TypeAdapter(float)


class ReferenceCall(Protocol):
    def __call__(self, *args: JsonValue, **kwargs: JsonValue) -> JsonValue: ...


class ReferenceModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


REFERENCE: ReferenceModule = importlib.import_module("litellm")


class RecordedCall(TypedDict):
    path: str
    headers: dict[str, str]
    body: bytes


class RerankServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(
        self,
        status: int = OK,
        body: dict[str, JsonValue] | None = None,
        delay: float = 0.0,
    ) -> None:
        self.status = status
        self.body = RERANK_BODY if body is None else body
        self.delay = delay
        self.calls: list[RecordedCall] = []
        super().__init__((HOST, 0), _Handler)
        _Handler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}/v1/rerank"

    def record(self, path: str, headers: dict[str, str], body: bytes) -> None:
        self.calls.append({"path": path, "headers": headers, "body": body})

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class _Handler(BaseHTTPRequestHandler):
    owner: ClassVar[RerankServer]

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        body = b"" if declared is None else self.rfile.read(int(declared))
        self.owner.record(self.path, dict(self.headers.items()), body)
        time.sleep(self.owner.delay)
        content = json.dumps(self.owner.body).encode()
        self.send_response(self.owner.status)
        self.send_header("Content-Type", JSON_TYPE)
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)


def parameter_rows(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        default: DefaultView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(default.default)))
    return rows


def dumped_of(response: RerankResponse) -> dict[str, JsonValue]:
    dumped = response.model_dump()
    return DUMP.validate_python(dumped)


def results_of(response: RerankResponse) -> list[dict[str, JsonValue]]:
    dumped = dumped_of(response)
    return RECORDS.validate_python(dumped["results"])


def hidden_of(response: RerankResponse) -> dict[str, JsonValue]:
    private: dict[str, JsonValue] | None = response.__pydantic_private__
    assert private is not None
    return DUMP.validate_python(private["_hidden_params"])


def response_of(outcome: RerankResponse | Awaitable[RerankResponse]) -> RerankResponse:
    match outcome:
        case RerankResponse():
            return outcome
        case Awaitable():
            raise AssertionError("a plain call answered a coroutine")


def body_of(call: RecordedCall) -> dict[str, JsonValue]:
    body = call["body"]
    return DUMP.validate_json(body)


def header_of(call: RecordedCall, name: str) -> str | None:
    lowered = name.lower()
    for header, value in call["headers"].items():
        if header.lower() == lowered:
            return value
    return None


def indexes_of(response: RerankResponse) -> list[JsonValue]:
    results = results_of(response)
    return [result["index"] for result in results]


async def flagged_rerank(base: str) -> RerankResponse:
    outcome = rerank(
        model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT, custom_llm_provider=PROVIDER,
        api_base=base, api_key=GOOD_KEY, _is_async=True,
    )
    match outcome:
        case RerankResponse():
            raise AssertionError("_is_async answered a response, not a coroutine")
        case Awaitable():
            pending = outcome
            assert inspect.iscoroutine(outcome)
            return await pending


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(rerank)) == parameter_rows(
        inspect.signature(REFERENCE.rerank)
    )
    assert parameter_rows(inspect.signature(arerank)) == parameter_rows(
        inspect.signature(REFERENCE.arerank)
    )
    assert not inspect.iscoroutinefunction(rerank)
    assert inspect.iscoroutinefunction(arerank)


def test_call_keeps_order() -> None:
    with closing(RerankServer()) as server:
        outcome = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, api_base=server.url, api_key=GOOD_KEY,
            litellm_call_id=CALL_ID,
        )
        call = server.calls[-1]
        response = response_of(outcome)
        hidden = hidden_of(response)
    assert indexes_of(response) == [SECOND_INDEX, FIRST_INDEX]
    assert [result["relevance_score"] for result in results_of(response)] == [
        SECOND_SCORE,
        FIRST_SCORE,
    ]
    assert results_of(response)[0]["document"] == {"text": "second"}
    assert dumped_of(response)["id"] == "rerank-1"
    assert header_of(call, "authorization") == f"Bearer {GOOD_KEY}"
    assert hidden["custom_llm_provider"] == PROVIDER
    assert hidden["litellm_call_id"] == CALL_ID
    assert hidden["litellm_model_name"] == MODEL
    assert hidden["model_id"] is None
    assert hidden["api_base"] == server.url
    assert DUMP.validate_python(hidden["additional_headers"]) == {}
    assert FLOAT.validate_python(hidden["response_cost"]) == pytest.approx(SEARCH_COST)
    assert "litellm_overhead_time_ms" not in hidden


def test_call_async_arerank() -> None:
    with closing(RerankServer()) as server:
        response = asyncio.run(
            arerank(
                model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
                custom_llm_provider=PROVIDER, api_base=server.url, api_key=GOOD_KEY,
            )
        )
        call = server.calls[-1]
        hidden = hidden_of(response)
    assert indexes_of(response) == [SECOND_INDEX, FIRST_INDEX]
    assert hidden["custom_llm_provider"] == PROVIDER
    assert FLOAT.validate_python(hidden["litellm_overhead_time_ms"]) >= 0.0
    assert FLOAT.validate_python(hidden["response_cost"]) == pytest.approx(SEARCH_COST)
    assert "return_documents" not in body_of(call)


def test_call_async_flag() -> None:
    with closing(RerankServer()) as server:
        response = asyncio.run(flagged_rerank(server.url))
    assert indexes_of(response) == [SECOND_INDEX, FIRST_INDEX]


def test_call_overhead_no_provider() -> None:
    with closing(RerankServer(delay=PROVIDER_DELAY)) as server:
        response = asyncio.run(
            arerank(
                model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
                custom_llm_provider=PROVIDER, api_base=server.url, api_key=GOOD_KEY,
            )
        )
        hidden = hidden_of(response)
    response_ms = FLOAT.validate_python(hidden["_response_ms"])
    overhead = FLOAT.validate_python(hidden["litellm_overhead_time_ms"])
    assert response_ms >= PROVIDER_DELAY * 1000.0
    assert overhead < response_ms / 2.0


def test_call_no_document() -> None:
    with closing(RerankServer(body=NO_DOCUMENT_BODY)) as server:
        outcome = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, return_documents=True,
            api_base=server.url, api_key=GOOD_KEY,
        )
        records = results_of(response_of(outcome))
    assert len(records) == 1
    assert records[0]["index"] == FIRST_INDEX
    assert "document" not in records[0]


def test_request_body() -> None:
    with closing(RerankServer()) as server:
        outcome = rerank(
            model=f"{PROVIDER}/{MODEL}", query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, top_n=0, rank_fields=RANK_FIELDS,
            max_chunks_per_doc=MAX_CHUNKS, api_base=server.url, api_key=GOOD_KEY,
        )
        body = body_of(server.calls[-1])
    assert body["model"] == MODEL
    assert body["query"] == QUERY
    assert body["documents"] == DOCUMENTS_TEXT
    assert body["top_n"] == 0
    assert body["rank_fields"] == RANK_FIELDS
    assert body["return_documents"] is True
    assert body["max_chunks_per_doc"] == MAX_CHUNKS
    assert "max_tokens_per_doc" not in body
    assert response_of(outcome).id == "rerank-1"


@pytest.mark.parametrize("return_documents", [False, None])
def test_request_return_documents(*, return_documents: bool | None) -> None:
    with closing(RerankServer()) as server:
        outcome = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, return_documents=return_documents,
            api_base=server.url, api_key=GOOD_KEY,
        )
        body = body_of(server.calls[-1])
    if return_documents is None:
        assert "return_documents" not in body
    else:
        assert body["return_documents"] is return_documents
    assert response_of(outcome).id == "rerank-1"


def test_request_unknown_kwargs() -> None:
    with closing(RerankServer()) as server:
        first = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, api_base=server.url, api_key=GOOD_KEY,
            truncate=True, instruction="pick one",
        )
        second = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, api_base=server.url, api_key=GOOD_KEY,
            litellm_logging_obj=object(),
        )
        extras = body_of(server.calls[0])
        logged = body_of(server.calls[1])
    assert "truncate" not in extras
    assert "instruction" not in extras
    assert logged["query"] == QUERY
    assert response_of(first).id == "rerank-1"
    assert response_of(second).id == "rerank-1"


def test_request_headers() -> None:
    with closing(RerankServer()) as server:
        outcome = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, api_base=server.url, api_key=GOOD_KEY,
            headers={"x-one": "1", "x-two": "2"},
        )
        call = server.calls[-1]
        body = body_of(call)
    assert header_of(call, "x-one") == "1"
    assert header_of(call, "x-two") == "2"
    assert header_of(call, "authorization") == f"Bearer {GOOD_KEY}"
    assert "headers" not in body
    assert body["query"] == QUERY
    assert response_of(outcome).id == "rerank-1"


def test_request_infinity_key(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("INFINITY_API_KEY", raising=False)
    original = betterlite.infinity_key
    betterlite.infinity_key = INFINITY_KEY
    try:
        with closing(RerankServer()) as server:
            outcome = rerank(
                model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
                custom_llm_provider=INFINITY_PROVIDER, api_base=server.url,
            )
            call = server.calls[-1]
        assert header_of(call, "authorization") == f"Bearer {INFINITY_KEY}"
        assert response_of(outcome).id == "rerank-1"
    finally:
        betterlite.infinity_key = original


LIMIT_CASES: Final[
    list[tuple[list[str | dict[str, JsonValue]], dict[str, str] | None, str]]
] = [
    ([DOCUMENTS_TEXT[0]] * (MAX_DOCUMENTS + 1), None, "documents"),
    (DOCUMENTS_TEXT, {f"h{index}": "v" for index in range(MAX_HEADERS + 1)}, "headers"),
]


@pytest.mark.parametrize(("documents", "headers", "message"), LIMIT_CASES)
def test_request_caps_refused(
    documents: list[str | dict[str, JsonValue]],
    headers: dict[str, str] | None,
    message: str,
) -> None:
    with closing(RerankServer()) as server:
        with pytest.raises(ValueError, match=message):
            _ = rerank(
                model=MODEL, query=QUERY, documents=documents,
                custom_llm_provider=PROVIDER, api_base=server.url, api_key=GOOD_KEY,
                headers=headers,
            )
        assert server.calls == []


@pytest.mark.parametrize(("documents", "headers", "message"), LIMIT_CASES)
def test_async_request_caps(
    documents: list[str | dict[str, JsonValue]],
    headers: dict[str, str] | None,
    message: str,
) -> None:
    with closing(RerankServer()) as server:
        with pytest.raises(ValueError, match=message):
            _ = asyncio.run(
                arerank(
                    model=MODEL, query=QUERY, documents=documents,
                    custom_llm_provider=PROVIDER, api_base=server.url, api_key=GOOD_KEY,
                    headers=headers,
                )
            )
        assert server.calls == []


def test_request_headers_limit() -> None:
    module_headers = {f"h{index}": "v" for index in range(MAX_HEADERS + 1)}
    with temporary_headers(module_headers), pytest.raises(ValueError, match="headers"):
        _ = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER,
        )


@contextmanager
def temporary_headers(headers: dict[str, str]) -> Iterator[None]:
    original = betterlite.headers
    betterlite.headers = headers  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L593
    try:
        yield
    finally:
        betterlite.headers = original


def test_request_bytes_refused() -> None:
    documents: list[str | dict[str, JsonValue]] = ["x" * (WIRE_LIMIT + 1)]
    with closing(RerankServer()) as server:
        with pytest.raises(ValueError, match="bytes"):
            _ = rerank(
                model=MODEL, query=QUERY, documents=documents,
                custom_llm_provider=PROVIDER, api_base=server.url, api_key=GOOD_KEY,
            )
        assert server.calls == []


def test_async_request_bytes() -> None:
    documents: list[str | dict[str, JsonValue]] = ["x" * (WIRE_LIMIT + 1)]
    with closing(RerankServer()) as server:
        with pytest.raises(ValueError, match="bytes"):
            _ = asyncio.run(
                arerank(
                    model=MODEL, query=QUERY, documents=documents,
                    custom_llm_provider=PROVIDER, api_base=server.url, api_key=GOOD_KEY,
                )
            )
        assert server.calls == []


def test_error_authentication() -> None:
    with closing(RerankServer(status=AUTH_STATUS, body=AUTH_BODY)) as server, pytest.raises(
        AuthenticationError
    ) as caught:
        _ = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, api_base=server.url, api_key=BAD_KEY,
        )
    error = caught.value
    assert error.status_code == AUTH_STATUS
    assert error.llm_provider == PROVIDER
    assert AUTH_TEXT in error.message
    cause = error.__cause__
    assert cause is not None
    assert cause.__class__.__name__ == "NativeRerankError"
    assert "AuthenticationError" in str(cause)


def test_error_internal_server() -> None:
    with closing(RerankServer(status=SERVER_ERROR_STATUS)) as server, pytest.raises(
        InternalServerError
    ) as caught:
        _ = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, api_base=server.url, api_key=GOOD_KEY,
        )
    assert caught.value.status_code == SERVER_ERROR_STATUS


def test_error_model_none() -> None:
    with pytest.raises(BadRequestError) as caught:
        _ = rerank(model=None, query=QUERY, documents=DOCUMENTS_TEXT)
    error = caught.value
    assert error.status_code == BAD_REQUEST_STATUS
    assert error.model is None
    assert error.llm_provider == ""
    assert MISSING_MODEL_TEXT in error.message
    with pytest.raises(BadRequestError) as async_caught:
        _ = asyncio.run(arerank(model=None, query=QUERY, documents=DOCUMENTS_TEXT))
    assert async_caught.value.status_code == BAD_REQUEST_STATUS


@pytest.mark.parametrize(
    ("provider", "message"),
    [("openai", UNSUPPORTED_TEXT), ("together_ai", NOT_WIRED_TEXT)],
)
def test_error_provider_refused(provider: str, message: str) -> None:
    with closing(RerankServer()) as server, pytest.raises(APIConnectionError) as caught:
        _ = rerank(
            model="x", query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=provider, api_base=server.url, api_key=GOOD_KEY,
        )
    assert server.calls == []
    assert caught.value.status_code == SERVER_ERROR_STATUS
    assert message in caught.value.message


def test_error_client_refused() -> None:
    with pytest.raises(UnsupportedCallArgument) as refused:
        _ = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, client=object(),
        )
    assert refused.value.argument == "client"


def test_error_session_refused() -> None:
    with pytest.raises(UnsupportedCallArgument) as refused:
        _ = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, shared_session=object(),
        )
    assert refused.value.argument == "shared_session"


def test_error_azure_refused() -> None:
    with pytest.raises(UnsupportedCallArgument) as refused:
        _ = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, azure_ad_token_provider=object(),
        )
    assert refused.value.argument == "azure_ad_token_provider"


@pytest.mark.parametrize("entry", ["sync", "async", "flag"])
@pytest.mark.parametrize("field", ["documents", "query", "extra", "aggregate", "settings"])
def test_request_before_copy(entry: str, field: str) -> None:
    text = "x" * (WIRE_LIMIT + 1)
    query = text if field == "query" else QUERY
    documents: list[str | dict[str, JsonValue]] = [text] if field == "documents" else []
    extra: JsonValue = {"nested": text} if field == "extra" else None
    if field == "aggregate":
        query = "x" * (WIRE_LIMIT // 2)
        documents = [{"text": query}]
    original = betterlite.api_base
    original_validator = rerank_module.JSON
    rerank_module.JSON = RefuseValidation()
    try:
        if field == "settings":
            betterlite.api_base = text
        with pytest.raises(ValueError, match="bytes"):
            size_probe(entry, query, documents, extra)
    finally:
        betterlite.api_base = original
        rerank_module.JSON = original_validator


def size_probe(
    entry: str, query: str, documents: list[str | dict[str, JsonValue]], extra: JsonValue,
) -> None:
    if entry == "async":
        _ = asyncio.run(arerank(MODEL, query, documents, extra=extra))
    else:
        _ = rerank(MODEL, query, documents, _is_async=entry == "flag", extra=extra)


class RefuseValidation(JsonValidator):
    @override
    def validate_python(self, value: CallValue, /) -> JsonValue:
        raise AssertionError("validation ran before the input limit")


@pytest.mark.parametrize("entry", ["sync", "async"])
@pytest.mark.parametrize("source", ["caller", "module", "combined"])
def test_request_before_headers(entry: str, source: str) -> None:
    caller: dict[str, str] = {}
    module: dict[str, str] = {}
    if source == "caller":
        caller = {f"h{index}": "v" for index in range(MAX_HEADERS + 1)}
    else:
        module = {f"h{index}": "v" for index in range(MAX_HEADERS)}
        if source == "module":
            module["extra"] = "v"
        else:
            caller["extra"] = "v"
    original = rerank_module.JSON
    with temporary_headers(module):
        rerank_module.JSON = RefuseValidation()
        try:
            if entry == "async":
                with pytest.raises(ValueError, match="headers"):
                    _ = asyncio.run(arerank(MODEL, QUERY, [], headers=caller))
            else:
                with pytest.raises(ValueError, match="headers"):
                    _ = rerank(MODEL, QUERY, [], headers=caller)
        finally:
            rerank_module.JSON = original


@pytest.mark.parametrize("value", [
    None, True, False, 0, -123, 1.25, "", "ascii", '"\\\n\b\t\r\f\x00\x1f\x7f',
    "é中😀", [], {}, [None, True, {"nested": ["é", 42]}],
])
def test_request_size_matches_json(value: JsonValue) -> None:
    budget = RequestBudget()
    budget.consume(value)
    assert budget.remaining == MAX_REQUEST_BYTES - len(json.dumps(value))


@pytest.mark.parametrize("delta", [-1, 0, 1])
def test_request_size_boundary(delta: int) -> None:
    text = "x" * (WIRE_LIMIT - len('""') + delta)
    budget = RequestBudget()
    if delta > 0:
        with pytest.raises(ValueError, match="bytes"):
            budget.consume(text)
    else:
        budget.consume(text)
        assert budget.remaining == -delta


def test_request_size_escaped() -> None:
    text = "😀" * (WIRE_LIMIT // len("\\ud83d\\ude00") + 1)
    budget = RequestBudget()
    with pytest.raises(ValueError, match="bytes"):
        budget.consume({"nested": [text]})


def test_request_size_cycle() -> None:
    cyclic: list[CallValue] = []
    cyclic.append(cyclic)
    budget = RequestBudget()
    with pytest.raises(ValueError, match="nesting"):
        budget.consume(cyclic)


class ChunkedText(str):
    slices: int = 0

    @override
    def __getitem__(self, key: SupportsIndex | slice, /) -> str:
        result = super().__getitem__(key)
        assert len(result) <= STRING_CHUNK
        self.slices += 1
        return result


def test_request_encoding_chunks() -> None:
    budget = RequestBudget()
    text = ChunkedText("é" * (STRING_CHUNK + 1))
    budget.consume(text)
    assert budget.remaining == WIRE_LIMIT - len(json.dumps(text))
    assert text.slices == (len(text) + STRING_CHUNK - 1) // STRING_CHUNK


@pytest.mark.parametrize("depth", [MAX_DEPTH - 1, MAX_DEPTH, MAX_DEPTH + 1])
def test_request_depth_boundary(depth: int) -> None:
    budget = RequestBudget()
    expected = pytest.raises(ValueError, match="nesting") if depth > MAX_DEPTH else nullcontext()
    with expected:
        budget.consume(None, depth)
    assert budget.remaining == WIRE_LIMIT - (len("null") if depth <= MAX_DEPTH else 0)


@pytest.mark.parametrize("count", [0, MAX_HEADERS - 1, MAX_HEADERS])
def test_request_headers_boundary(count: int) -> None:
    with temporary_headers({}):
        check_request_size((), {"headers": {f"h{index}": "v" for index in range(count)}})


def test_request_integer_budget() -> None:
    budget = RequestBudget(remaining=1)
    with pytest.raises(ValueError, match="bytes"):
        budget.consume(1 << WIRE_LIMIT)
