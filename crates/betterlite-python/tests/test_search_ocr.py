import asyncio
import inspect
import json
import threading
from collections.abc import Callable
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final

import litellm
import litellm.exceptions as their_errors
import pytest
from litellm.search.cost_calculator import search_provider_cost_per_query as their_cost
from pydantic import BaseModel, JsonValue

from betterlite import exceptions
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite.ocr import aocr, ocr
from betterlite.search import asearch, search, search_provider_cost_per_query

HOST: Final = "127.0.0.1"
OK: Final = 200
RATE_LIMITED: Final = 429
SERVER_ERROR: Final = 500
API_KEY: Final = "test-key"
UNREACHABLE: Final = "http://127.0.0.1:9"
SEARCH_PROVIDER: Final = "brave"
OCR_MODEL: Final = "mistral/mistral-ocr-latest"
OCR_DOCUMENT: Final[dict[str, JsonValue]] = {
    "type": "document_url",
    "document_url": "data:application/pdf;base64,YWJj",
}
SEARCH_BODY: Final[dict[str, JsonValue]] = {
    "web": {
        "results": [
            {
                "title": "First result",
                "url": "https://example.test/first",
                "description": "About the first",
            },
            {
                "title": "Second result",
                "url": "https://example.test/second",
                "description": "About the second",
            },
        ]
    }
}
OCR_BODY: Final[dict[str, JsonValue]] = {
    "pages": [{"index": 0, "markdown": "native OCR response", "images": [], "dimensions": None}],
    "model": "mistral-ocr-latest",
    "usage_info": {"pages_processed": 1, "doc_size_bytes": 3},
}
ERROR_BODY: Final[dict[str, JsonValue]] = {"message": "the provider refused"}


class SearchOcrServer(ThreadingHTTPServer):
    daemon_threads = True
    status: int
    calls: list[tuple[str, str]]

    def __init__(self, status: int) -> None:
        super().__init__((HOST, 0), _Handler)
        self.status = status
        self.calls = []
        _Handler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}"

    def record(self, method: str, path: str) -> None:
        self.calls.append((method, path))

    # contextlib.closing calls close, so the serving thread stops with the test
    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class _Handler(BaseHTTPRequestHandler):
    owner: ClassVar[SearchOcrServer]

    def do_GET(self) -> None:
        self.owner.record("GET", self.path)
        payload = SEARCH_BODY if self.owner.status == OK else ERROR_BODY
        self.reply(payload, self.owner.status)

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        if declared is not None:
            self.rfile.read(int(declared))
        self.owner.record("POST", self.path)
        payload = OCR_BODY if self.owner.status == OK else ERROR_BODY
        self.reply(payload, self.owner.status)

    def reply(self, payload: dict[str, JsonValue], status: int) -> None:
        content = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)


def _parameters(function: Callable[..., object]) -> list[tuple[str, str]]:
    return [
        (parameter.name, str(parameter.kind))
        for parameter in inspect.signature(function).parameters.values()
    ]


# litellm.search and litellm.ocr may answer with a coroutine, so the test reads the value it got
def _model_dump(value: object) -> dict[str, JsonValue]:
    assert isinstance(value, BaseModel)
    return value.model_dump()


def test_search_signature_matches_litellm() -> None:
    theirs: Callable[..., object] = litellm.search
    assert _parameters(search) == _parameters(theirs)


def test_asearch_signature_matches_litellm() -> None:
    theirs: Callable[..., object] = litellm.asearch
    assert _parameters(asearch) == _parameters(theirs)


def test_search_cost_signature_matches_litellm() -> None:
    theirs: Callable[..., object] = their_cost
    assert _parameters(search_provider_cost_per_query) == _parameters(theirs)


def test_ocr_signature_matches_litellm() -> None:
    their_ocr: Callable[..., object] = litellm.ocr
    their_aocr: Callable[..., object] = litellm.aocr
    assert _parameters(ocr) == _parameters(their_ocr)
    assert _parameters(aocr) == _parameters(their_aocr)


def test_ocr_signature_refuses_extra_positional_arguments() -> None:
    with pytest.raises(TypeError) as ours:
        ocr(OCR_MODEL, OCR_DOCUMENT, "third")
    with pytest.raises(TypeError) as theirs:
        litellm.ocr(OCR_MODEL, OCR_DOCUMENT, "third")
    assert str(ours.value).replace("ocr()", "call()") == str(theirs.value).replace(
        "ocr()", "call()"
    )


def test_search_calls_match_litellm() -> None:
    with closing(SearchOcrServer(OK)) as server:
        ours = search(
            query="brave search",
            search_provider=SEARCH_PROVIDER,
            max_results=2,
            api_key=API_KEY,
            api_base=server.url,
        )
        theirs: object = litellm.search(
            query="brave search",
            search_provider=SEARCH_PROVIDER,
            max_results=2,
            api_key=API_KEY,
            api_base=server.url,
        )
    assert _model_dump(ours) == _model_dump(theirs)


def test_asearch_calls_match_litellm() -> None:
    with closing(SearchOcrServer(OK)) as server:
        ours = asyncio.run(
            asearch(
                query="brave search",
                search_provider=SEARCH_PROVIDER,
                api_key=API_KEY,
                api_base=server.url,
            )
        )
        theirs = asyncio.run(
            litellm.asearch(
                query="brave search",
                search_provider=SEARCH_PROVIDER,
                api_key=API_KEY,
                api_base=server.url,
            )
        )
    assert _model_dump(ours) == _model_dump(theirs)


def test_search_cost_calls_match_litellm() -> None:
    cases: list[tuple[str, str | None, int, dict[str, JsonValue] | None]] = [
        ("exa_ai/search", None, 1, None),
        ("exa_ai/search", "exa_ai", 3, {"max_results": 30}),
        ("exa_ai/search", "exa_ai", 1, {"max_results": 1000}),
    ]
    for model, provider, queries, optional_params in cases:
        ours = search_provider_cost_per_query(
            model,
            custom_llm_provider=provider,
            number_of_queries=queries,
            optional_params=optional_params,
        )
        theirs = their_cost(
            model,
            custom_llm_provider=provider,
            number_of_queries=queries,
            optional_params=optional_params,
        )
        assert ours == pytest.approx(theirs)


def test_ocr_calls_match_litellm() -> None:
    with closing(SearchOcrServer(OK)) as server:
        ours = ocr(
            model=OCR_MODEL,
            document=OCR_DOCUMENT,
            api_key=API_KEY,
            api_base=server.url,
        )
        theirs: object = litellm.ocr(
            model=OCR_MODEL,
            document=OCR_DOCUMENT,
            api_key=API_KEY,
            api_base=server.url,
        )
    assert _model_dump(ours) == _model_dump(theirs)


def test_aocr_calls_match_litellm() -> None:
    with closing(SearchOcrServer(OK)) as server:
        ours = asyncio.run(
            aocr(model=OCR_MODEL, document=OCR_DOCUMENT, api_key=API_KEY, api_base=server.url)
        )
        theirs = asyncio.run(
            litellm.aocr(
                model=OCR_MODEL, document=OCR_DOCUMENT, api_key=API_KEY, api_base=server.url
            )
        )
    assert _model_dump(ours) == _model_dump(theirs)


def test_ocr_provider_error_keeps_its_class() -> None:
    server = SearchOcrServer(RATE_LIMITED)
    try:
        with pytest.raises(exceptions.RateLimitError) as ours:
            ocr(model=OCR_MODEL, document=OCR_DOCUMENT, api_key=API_KEY, api_base=server.url)
        with pytest.raises(their_errors.RateLimitError) as theirs:
            litellm.ocr(
                model=OCR_MODEL, document=OCR_DOCUMENT, api_key=API_KEY, api_base=server.url
            )
    finally:
        server.close()
    assert type(ours.value).__name__ == type(theirs.value).__name__
    assert ours.value.status_code == theirs.value.status_code


def test_ocr_server_error_keeps_its_class() -> None:
    server = SearchOcrServer(SERVER_ERROR)
    try:
        with pytest.raises(exceptions.InternalServerError) as ours:
            ocr(model=OCR_MODEL, document=OCR_DOCUMENT, api_key=API_KEY, api_base=server.url)
        with pytest.raises(their_errors.InternalServerError) as theirs:
            litellm.ocr(
                model=OCR_MODEL, document=OCR_DOCUMENT, api_key=API_KEY, api_base=server.url
            )
    finally:
        server.close()
    assert type(ours.value).__name__ == type(theirs.value).__name__
    assert ours.value.status_code == theirs.value.status_code


def test_ocr_error_invalid_req_format_is_unsupported_params() -> None:
    arguments: Final[dict[str, JsonValue]] = {
        "model": OCR_MODEL,
        "document": OCR_DOCUMENT,
        "api_key": API_KEY,
        "api_base": UNREACHABLE,
        "req_format": "bogus",
        "num_retries": 0,
    }
    with pytest.raises(exceptions.UnsupportedParamsError) as ours:
        ocr(**arguments)
    with pytest.raises(their_errors.UnsupportedParamsError) as theirs:
        litellm.ocr(**arguments)
    assert ours.value.status_code == theirs.value.status_code
    for value in ("req_format", "bogus", "native", "litellm"):
        assert value in ours.value.message
        assert value in theirs.value.message


def test_ocr_error_document_without_type_is_a_bad_request() -> None:
    document: Final[dict[str, JsonValue]] = {"document_url": "https://example.test/a.pdf"}
    server = SearchOcrServer(OK)
    try:
        with pytest.raises(exceptions.BadRequestError) as ours:
            ocr(model=OCR_MODEL, document=document, api_key=API_KEY, api_base=server.url)
        with pytest.raises(their_errors.BadRequestError) as theirs:
            litellm.ocr(
                model=OCR_MODEL, document=document, api_key=API_KEY, api_base=server.url
            )
        calls = list(server.calls)
    finally:
        server.close()
    assert calls == []
    assert "type" in str(ours.value)
    assert theirs.value.status_code == ours.value.status_code


@pytest.mark.parametrize("name", ["client", "shared_session"])
def test_search_unsupported_client_object_is_refused(name: str) -> None:
    blocked: dict[str, JsonValue] = {name: "an object Rust cannot take"}
    with pytest.raises(UnsupportedCallArgument) as error:
        search(query="brave search", search_provider=SEARCH_PROVIDER, **blocked)
    assert error.value.argument in blocked


@pytest.mark.parametrize("name", ["client", "shared_session"])
def test_ocr_unsupported_shared_session_is_refused(name: str) -> None:
    blocked: dict[str, JsonValue] = {name: "an object Rust cannot take"}
    with pytest.raises(UnsupportedCallArgument) as error:
        ocr(model=OCR_MODEL, document=OCR_DOCUMENT, **blocked)
    assert error.value.argument in blocked
