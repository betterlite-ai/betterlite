import asyncio
import importlib
import inspect
import json
import threading
from collections.abc import Coroutine
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, Protocol

import pytest
from endpoint_server import (
    BAD_KEY,
    GOOD_KEY,
    HOST,
    JSON_TYPE,
    OK,
    RESPONSES,
    EndpointServer,
    RecordedCall,
)
from pydantic import JsonValue, TypeAdapter

from betterlite.exceptions import (
    AuthenticationError,
    BadRequestError,
    InternalServerError,
    UnsupportedParamsError,
)
from betterlite.main import aembedding, embedding
from betterlite.types.utils import EmbeddingResponse

GOOD_MODEL: Final = "text-embedding-3-small"
ADA_MODEL: Final = "text-embedding-ada-002"
PROVIDER: Final = "openai"
API_BASE_PATH: Final = "/v1"
EMBEDDINGS_PATH: Final = "/v1/embeddings"
AUTH_STATUS: Final = 401
BAD_REQUEST_STATUS: Final = 400
SERVER_ERROR_STATUS: Final = 500
PROMPT_TOKENS: Final = 2
COMPLETION_TOKENS: Final = 0
MOCK_PROMPT_TOKENS: Final = 10
DIMENSIONS: Final = 2
CUSTOM_COST: Final = 0.0001
CUSTOM_TOTAL: Final = 0.0002
CALL_ID: Final = "call-1"
USER: Final = "reader"
MOCK_ERROR_TEXT: Final = "Mock error"
BASE64_TEXT: Final = "AAAAAAAAAAA="
BARRIER_SECONDS: Final = 5.0
PARTIES: Final = 2
TOKENS_INPUT: Final[list[JsonValue]] = [1, 2]
TOKEN_LISTS_INPUT: Final[list[JsonValue]] = [[1, 2], [3, 4]]
STRINGS_INPUT: Final[list[JsonValue]] = ["one", "two"]
MOCK_VECTOR: Final[list[JsonValue]] = [0.1, 0.2]
FIXTURE_VECTOR: Final[list[JsonValue]] = [0.1, 0.2]
MISSING_MODEL_TEXT: Final = "model parameter is required but was None"
MISSING_KEY_TEXT: Final = "Missing credentials"
DIMENSIONS_TEXT: Final = "Setting dimensions is not supported"
AUTH_TEXT: Final = "Invalid API key provided: bad-key"
HEADERS_ONE: Final[dict[str, str]] = {"x-custom": "one"}
HEADERS_TWO: Final[dict[str, str]] = {"x-custom": "two"}

DUMP: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
RECORDS: Final[TypeAdapter[list[dict[str, JsonValue]]]] = TypeAdapter(
    list[dict[str, JsonValue]]
)
FLOAT: Final[TypeAdapter[float]] = TypeAdapter(float)


class ReferenceCall(Protocol):
    def __call__(self, *args: JsonValue, **kwargs: JsonValue) -> JsonValue: ...


class ReferenceModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


REFERENCE: ReferenceModule = importlib.import_module("litellm")


class EmbeddingHandler(BaseHTTPRequestHandler):
    parties: ClassVar[threading.Barrier]

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        if declared is not None:
            _ = self.rfile.read(int(declared))
        try:
            self.parties.wait(BARRIER_SECONDS)
        except threading.BrokenBarrierError:
            self.send_error(SERVER_ERROR_STATUS)
            return
        content = json.dumps(RESPONSES[EMBEDDINGS_PATH]).encode()
        self.send_response(OK)
        self.send_header("Content-Type", JSON_TYPE)
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)


class BarrierServer(ThreadingHTTPServer):
    daemon_threads = True
    parties: threading.Barrier

    def __init__(self) -> None:
        self.parties = threading.Barrier(PARTIES)
        super().__init__((HOST, 0), EmbeddingHandler)
        EmbeddingHandler.parties = self.parties
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}{API_BASE_PATH}"

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


def parameter_rows(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        default: inspect.Parameter = parameter
        rows.append((parameter.name, parameter.kind.name, repr(default.default)))
    return rows


def dumped_of(response: EmbeddingResponse) -> dict[str, JsonValue]:
    dumped = response.model_dump()
    return DUMP.validate_python(dumped)


def records_of(response: EmbeddingResponse) -> list[dict[str, JsonValue]]:
    dumped = dumped_of(response)
    return RECORDS.validate_python(dumped["data"])


def usage_of(response: EmbeddingResponse) -> dict[str, JsonValue]:
    dumped = dumped_of(response)
    return DUMP.validate_python(dumped["usage"])


def hidden_of(response: EmbeddingResponse) -> dict[str, JsonValue]:
    hidden = response["_hidden_params"]
    return DUMP.validate_python(hidden)


def response_of(
    outcome: EmbeddingResponse | Coroutine[object, object, EmbeddingResponse],
) -> EmbeddingResponse:
    match outcome:
        case EmbeddingResponse():
            return outcome
        case Coroutine():
            raise AssertionError("a plain call answered a coroutine")


async def settled(
    outcome: EmbeddingResponse | Coroutine[object, object, EmbeddingResponse],
) -> EmbeddingResponse:
    match outcome:
        case EmbeddingResponse():
            return outcome
        case Coroutine():
            return await outcome


def header_of(call: RecordedCall, name: str) -> str | None:
    lowered = name.lower()
    for header, value in call["headers"].items():
        if header.lower() == lowered:
            return value
    return None


def body_of(call: RecordedCall) -> dict[str, JsonValue]:
    body = call["body"]
    return DUMP.validate_json(body)


async def two_embeddings(base: str) -> list[EmbeddingResponse]:
    rows = await asyncio.gather(
        aembedding(model=GOOD_MODEL, input="one", api_base=base, api_key=GOOD_KEY),
        aembedding(model=GOOD_MODEL, input="two", api_base=base, api_key=GOOD_KEY),
    )
    return rows


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(embedding)) == parameter_rows(
        inspect.signature(REFERENCE.embedding)
    )
    assert parameter_rows(inspect.signature(aembedding)) == parameter_rows(
        inspect.signature(REFERENCE.aembedding)
    )
    assert not inspect.iscoroutinefunction(embedding)
    assert inspect.iscoroutinefunction(aembedding)


def test_call_string_input() -> None:
    with closing(EndpointServer()) as server:
        response = response_of(
            embedding(
                model=GOOD_MODEL,
                input="hi",
                api_base=server.url + API_BASE_PATH,
                api_key=GOOD_KEY,
            )
        )
        call = server.calls[-1]
    assert dumped_of(response)["model"] == GOOD_MODEL
    assert records_of(response)[0]["embedding"] == FIXTURE_VECTOR
    assert body_of(call)["input"] == "hi"
    assert header_of(call, "authorization") == f"Bearer {GOOD_KEY}"


def test_call_list_input() -> None:
    with closing(EndpointServer()) as server:
        outcome = embedding(
            model=GOOD_MODEL,
            input=STRINGS_INPUT,
            api_base=server.url + API_BASE_PATH,
            api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    assert body_of(call)["input"] == STRINGS_INPUT
    assert response_of(outcome).model_dump()["object"] == "list"


def test_call_token_input() -> None:
    with closing(EndpointServer()) as server:
        outcome = embedding(
            model=GOOD_MODEL,
            input=TOKENS_INPUT,
            api_base=server.url + API_BASE_PATH,
            api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    assert body_of(call)["input"] == TOKENS_INPUT
    assert usage_of(response_of(outcome))["prompt_tokens"] == PROMPT_TOKENS


def test_call_token_list_input() -> None:
    with closing(EndpointServer()) as server:
        outcome = embedding(
            model=GOOD_MODEL,
            input=TOKEN_LISTS_INPUT,
            api_base=server.url + API_BASE_PATH,
            api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    assert body_of(call)["input"] == TOKEN_LISTS_INPUT
    assert body_of(call)["model"] == GOOD_MODEL
    assert response_of(outcome).model_dump()["object"] == "list"


def test_call_default_input() -> None:
    with closing(EndpointServer()) as server:
        outcome = embedding(
            model=GOOD_MODEL,
            api_base=server.url + API_BASE_PATH,
            api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    assert body_of(call)["input"] == []
    assert response_of(outcome).model_dump()["model"] == GOOD_MODEL


def test_call_body_fields() -> None:
    with closing(EndpointServer()) as server:
        outcome = embedding(
            model=GOOD_MODEL,
            input="hi",
            dimensions=DIMENSIONS,
            encoding_format="float",
            user=USER,
            api_base=server.url + API_BASE_PATH,
            api_key=GOOD_KEY,
        )
        body = body_of(server.calls[-1])
    assert body["dimensions"] == DIMENSIONS
    assert body["encoding_format"] == "float"
    assert body["user"] == USER
    assert body["input"] == "hi"
    assert response_of(outcome).model_dump()["model"] == GOOD_MODEL


def test_call_headers() -> None:
    with closing(EndpointServer()) as server:
        outcome = embedding(
            model=GOOD_MODEL,
            input="hi",
            headers=HEADERS_ONE,
            extra_headers=HEADERS_TWO,
            api_base=server.url + API_BASE_PATH,
            api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    assert header_of(call, "x-custom") == "two"
    assert header_of(call, "authorization") == f"Bearer {GOOD_KEY}"
    assert response_of(outcome).model_dump()["model"] == GOOD_MODEL


def test_call_hidden_params() -> None:
    server = EndpointServer()
    try:
        base = server.url + API_BASE_PATH
        outcome = embedding(
            model=GOOD_MODEL,
            input="hi",
            litellm_call_id=CALL_ID,
            api_base=base,
            api_key=GOOD_KEY,
        )
    finally:
        server.close()
    hidden = hidden_of(response_of(outcome))
    headers = DUMP.validate_python(hidden["additional_headers"])
    assert hidden["custom_llm_provider"] == PROVIDER
    assert hidden["model_id"] is None
    assert hidden["litellm_model_name"] == GOOD_MODEL
    assert hidden["litellm_call_id"] == CALL_ID
    assert hidden["api_base"] == base
    assert headers["llm_provider-content-type"] == JSON_TYPE
    assert FLOAT.validate_python(hidden["response_cost"]) > 0.0


def test_call_custom_cost() -> None:
    with closing(EndpointServer()) as server:
        outcome = embedding(
            model=GOOD_MODEL,
            input="hi",
            input_cost_per_token=CUSTOM_COST,
            output_cost_per_token=CUSTOM_COST,
            api_base=server.url + API_BASE_PATH,
            api_key=GOOD_KEY,
        )
        hidden = hidden_of(response_of(outcome))
    assert FLOAT.validate_python(hidden["response_cost"]) == pytest.approx(CUSTOM_TOTAL)


def test_call_mock_vector() -> None:
    response = response_of(
        embedding(model=GOOD_MODEL, input="hi", mock_response=MOCK_VECTOR)
    )
    hidden = hidden_of(response)
    assert records_of(response)[0]["embedding"] == MOCK_VECTOR
    assert usage_of(response)["prompt_tokens"] == MOCK_PROMPT_TOKENS
    assert usage_of(response)["total_tokens"] == COMPLETION_TOKENS
    assert hidden["model_id"] is None
    assert "custom_llm_provider" not in hidden


def test_call_mock_base64() -> None:
    response = response_of(
        embedding(model=GOOD_MODEL, input="hi", mock_response=BASE64_TEXT)
    )
    assert records_of(response)[0]["embedding"] == BASE64_TEXT


def test_call_provider_model() -> None:
    with closing(EndpointServer()) as server:
        outcome = embedding(
            model=f"{PROVIDER}/{GOOD_MODEL}",
            input="hi",
            api_base=server.url + API_BASE_PATH,
            api_key=GOOD_KEY,
        )
        body = body_of(server.calls[-1])
    assert body["model"] == GOOD_MODEL
    assert response_of(outcome).model_dump()["model"] == GOOD_MODEL


def test_error_unauthorized() -> None:
    with closing(EndpointServer()) as server, pytest.raises(AuthenticationError) as caught:
        embedding(
            model=GOOD_MODEL,
            input="hi",
            api_base=server.url + API_BASE_PATH,
            api_key=BAD_KEY,
        )
    error = caught.value
    assert error.status_code == AUTH_STATUS
    assert error.model == GOOD_MODEL
    assert error.llm_provider == PROVIDER
    assert AUTH_TEXT in error.message
    cause = error.__cause__
    assert cause is not None
    assert cause.__class__.__name__ == "NativeEmbeddingError"
    assert "AuthenticationError" in str(cause)


def test_error_dimensions() -> None:
    with closing(EndpointServer()) as server, pytest.raises(UnsupportedParamsError) as caught:
        embedding(
            model=ADA_MODEL,
            input="hi",
            dimensions=DIMENSIONS,
            api_base=server.url + API_BASE_PATH,
            api_key=GOOD_KEY,
        )
    assert server.calls == []
    assert caught.value.status_code == BAD_REQUEST_STATUS
    assert DIMENSIONS_TEXT in caught.value.message


def test_error_dimensions_dropped() -> None:
    with closing(EndpointServer()) as server:
        outcome = embedding(
            model=ADA_MODEL,
            input="hi",
            dimensions=DIMENSIONS,
            drop_params=True,
            api_base=server.url + API_BASE_PATH,
            api_key=GOOD_KEY,
        )
        body = body_of(server.calls[-1])
    assert "dimensions" not in body
    assert response_of(outcome).model_dump()["model"] == GOOD_MODEL


def test_error_missing_key(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    monkeypatch.delenv("OPENAI_BASE_URL", raising=False)
    monkeypatch.delenv("OPENAI_API_BASE", raising=False)
    with closing(EndpointServer()) as server, pytest.raises(InternalServerError) as caught:
        embedding(model=GOOD_MODEL, input="hi", api_base=server.url + API_BASE_PATH)
    error = caught.value
    assert error.status_code == SERVER_ERROR_STATUS
    assert error.model == GOOD_MODEL
    assert error.llm_provider == PROVIDER
    assert MISSING_KEY_TEXT in error.message


def test_error_model_none() -> None:
    with pytest.raises(BadRequestError) as caught:
        embedding(model=None, input="hi")
    error = caught.value
    assert error.status_code == BAD_REQUEST_STATUS
    assert error.model is None
    assert error.llm_provider == ""
    assert MISSING_MODEL_TEXT in error.message


def test_error_mock_text() -> None:
    with pytest.raises(Exception, match=MOCK_ERROR_TEXT) as caught:
        embedding(model=GOOD_MODEL, input="hi", mock_response="error")
    assert caught.value.__class__ is Exception
    assert str(caught.value) == MOCK_ERROR_TEXT


def test_async_aembedding() -> None:
    with closing(EndpointServer()) as server:
        response = asyncio.run(
            aembedding(
                model=GOOD_MODEL,
                input="hi",
                api_base=server.url + API_BASE_PATH,
                api_key=GOOD_KEY,
            )
        )
    assert dumped_of(response)["model"] == GOOD_MODEL
    assert records_of(response)[0]["embedding"] == MOCK_VECTOR


def test_async_flag_coroutine() -> None:
    with closing(EndpointServer()) as server:
        flagged = embedding(
            model=GOOD_MODEL,
            input="hi",
            api_base=server.url + API_BASE_PATH,
            api_key=GOOD_KEY,
            aembedding=True,
        )
        assert inspect.isawaitable(flagged)
        response = asyncio.run(settled(flagged))
    assert dumped_of(response)["model"] == GOOD_MODEL


def test_async_runs_concurrently() -> None:
    server = BarrierServer()
    try:
        responses = asyncio.run(two_embeddings(server.url))
    finally:
        server.close()
    assert [dumped_of(response)["model"] for response in responses] == [
        GOOD_MODEL,
        GOOD_MODEL,
    ]


def test_async_model_before_await() -> None:
    pending = aembedding(model=None, input="hi")
    assert inspect.iscoroutine(pending)
    with pytest.raises(BadRequestError) as caught:
        asyncio.run(pending)
    assert caught.value.status_code == BAD_REQUEST_STATUS
    assert MISSING_MODEL_TEXT in caught.value.message
