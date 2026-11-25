import asyncio
import importlib
import inspect
import json
import threading
from collections.abc import Awaitable
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, Protocol, TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter

from betterlite._completion.errors import UnsupportedCallArgument
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
        self, status: int = OK, body: dict[str, JsonValue] | None = None
    ) -> None:
        self.status = status
        self.body = RERANK_BODY if body is None else body
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


def test_request_return_false() -> None:
    with closing(RerankServer()) as server:
        outcome = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, return_documents=False,
            api_base=server.url, api_key=GOOD_KEY,
        )
        body = body_of(server.calls[-1])
    assert body["return_documents"] is False
    assert response_of(outcome).id == "rerank-1"


def test_request_return_none() -> None:
    with closing(RerankServer()) as server:
        outcome = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, return_documents=None,
            api_base=server.url, api_key=GOOD_KEY,
        )
        body = body_of(server.calls[-1])
    assert "return_documents" not in body
    assert response_of(outcome).id == "rerank-1"


def test_request_extra_kwargs() -> None:
    with closing(RerankServer()) as server:
        outcome = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, api_base=server.url, api_key=GOOD_KEY,
            truncate=True, instruction="pick one",
        )
        body = body_of(server.calls[-1])
    assert "truncate" not in body
    assert "instruction" not in body
    assert response_of(outcome).id == "rerank-1"


def test_request_headers() -> None:
    with closing(RerankServer()) as server:
        outcome = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, api_base=server.url, api_key=GOOD_KEY,
            headers={"x-one": "1"},
        )
        call = server.calls[-1]
    assert header_of(call, "x-one") == "1"
    assert header_of(call, "authorization") == f"Bearer {GOOD_KEY}"
    assert response_of(outcome).id == "rerank-1"


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


def test_error_unsupported() -> None:
    with closing(RerankServer()) as server, pytest.raises(APIConnectionError) as caught:
        _ = rerank(
            model="x", query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider="openai", api_base=server.url, api_key=GOOD_KEY,
        )
    assert server.calls == []
    assert caught.value.status_code == SERVER_ERROR_STATUS
    assert UNSUPPORTED_TEXT in caught.value.message


def test_error_not_wired() -> None:
    with closing(RerankServer()) as server, pytest.raises(APIConnectionError) as caught:
        _ = rerank(
            model="rerank", query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider="together_ai", api_base=server.url, api_key=GOOD_KEY,
        )
    assert server.calls == []
    assert caught.value.status_code == SERVER_ERROR_STATUS
    assert NOT_WIRED_TEXT in caught.value.message


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


def test_request_logger_ignored() -> None:
    with closing(RerankServer()) as server:
        outcome = rerank(
            model=MODEL, query=QUERY, documents=DOCUMENTS_TEXT,
            custom_llm_provider=PROVIDER, api_base=server.url, api_key=GOOD_KEY,
            litellm_logging_obj=object(),
        )
        body = body_of(server.calls[-1])
    assert body["query"] == QUERY
    assert response_of(outcome).id == "rerank-1"
