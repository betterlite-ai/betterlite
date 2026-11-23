import asyncio
import importlib
import inspect
import json
import threading
from collections.abc import Coroutine
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, Protocol, TypeAlias, runtime_checkable

import pytest
from pydantic import JsonValue

from betterlite import exceptions
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite.ocr import aocr, ocr
from betterlite.search import asearch, search, search_provider_cost_per_query

HOST: Final = "127.0.0.1"
OK: Final = 200
RATE_LIMITED: Final = 429
SERVER_ERROR: Final = 500
GOOD_KEY: Final = "test-key"
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

CallArgument: TypeAlias = JsonValue


@runtime_checkable
class Dumpable(Protocol):
    def model_dump(self) -> dict[str, JsonValue]: ...


@runtime_checkable
class ErrorFields(Protocol):
    message: str
    status_code: int


class ReferenceCall(Protocol):
    def __call__(self, *args: CallArgument, **kwargs: CallArgument) -> Dumpable: ...


class ReferenceAsyncCall(Protocol):
    def __call__(
        self, *args: CallArgument, **kwargs: CallArgument
    ) -> Coroutine[None, None, Dumpable]: ...


class Reference(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


class ReferenceAsync(Protocol):
    def __getattr__(self, name: str) -> ReferenceAsyncCall: ...


class ReferenceErrors(Protocol):
    def __getattr__(self, name: str) -> type[BaseException]: ...


class CostCall(Protocol):
    def __call__(
        self,
        model: str,
        custom_llm_provider: str | None = None,
        number_of_queries: int = 1,
        optional_params: dict[str, JsonValue] | None = None,
    ) -> tuple[float, float]: ...


class ReferenceCost(Protocol):
    def __getattr__(self, name: str) -> CostCall: ...


# litellm's search and ocr decorators leave their functions untyped, so the reference keeps the declared shape
REFERENCE: Reference = importlib.import_module("litellm")
REFERENCE_ASYNC: ReferenceAsync = importlib.import_module("litellm")
REFERENCE_ERRORS: ReferenceErrors = importlib.import_module("litellm.exceptions")
REFERENCE_COST: ReferenceCost = importlib.import_module("litellm.search.cost_calculator")


class SearchOcrServer(ThreadingHTTPServer):
    daemon_threads = True
    status: int
    calls: list[tuple[str, str]]
    thread: threading.Thread

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


def _parameters(signature: inspect.Signature) -> list[tuple[str, str]]:
    return [
        (parameter.name, str(parameter.kind))
        for parameter in signature.parameters.values()
    ]


def error_fields(error: BaseException) -> ErrorFields:
    match error:
        case ErrorFields():
            return error
        case object():
            raise AssertionError("the error does not carry the fields")


def test_search_signature() -> None:
    assert _parameters(inspect.signature(search)) == _parameters(
        inspect.signature(REFERENCE.search)
    )


def test_asearch_signature() -> None:
    assert _parameters(inspect.signature(asearch)) == _parameters(
        inspect.signature(REFERENCE_ASYNC.asearch)
    )


def test_cost_signature() -> None:
    ours = inspect.signature(search_provider_cost_per_query)
    theirs = inspect.signature(REFERENCE_COST.search_provider_cost_per_query)
    assert _parameters(ours) == _parameters(theirs)


def test_ocr_signature() -> None:
    assert _parameters(inspect.signature(ocr)) == _parameters(inspect.signature(REFERENCE.ocr))
    assert _parameters(inspect.signature(aocr)) == _parameters(
        inspect.signature(REFERENCE_ASYNC.aocr)
    )


def test_ocr_signature_extra_args() -> None:
    with pytest.raises(TypeError) as ours:
        ocr(OCR_MODEL, OCR_DOCUMENT, "third")
    with pytest.raises(TypeError) as theirs:
        REFERENCE.ocr(OCR_MODEL, OCR_DOCUMENT, "third")
    ours_text = str(ours.value).replace("ocr()", "call()")
    theirs_text = str(theirs.value).replace("ocr()", "call()")
    assert ours_text == theirs_text


def test_search_calls() -> None:
    with closing(SearchOcrServer(OK)) as server:
        ours = search(
            query="brave search",
            search_provider=SEARCH_PROVIDER,
            max_results=2,
            api_key=GOOD_KEY,
            api_base=server.url,
        )
        theirs = REFERENCE.search(
            query="brave search",
            search_provider=SEARCH_PROVIDER,
            max_results=2,
            api_key=GOOD_KEY,
            api_base=server.url,
        ).model_dump()
    assert ours.model_dump() == theirs


def test_asearch_calls() -> None:
    with closing(SearchOcrServer(OK)) as server:
        ours = asyncio.run(
            asearch(
                query="brave search",
                search_provider=SEARCH_PROVIDER,
                api_key=GOOD_KEY,
                api_base=server.url,
            )
        )
        theirs = asyncio.run(
            REFERENCE_ASYNC.asearch(
                query="brave search",
                search_provider=SEARCH_PROVIDER,
                api_key=GOOD_KEY,
                api_base=server.url,
            )
        ).model_dump()
    assert ours.model_dump() == theirs


def test_cost_calls() -> None:
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
        theirs = REFERENCE_COST.search_provider_cost_per_query(
            model,
            custom_llm_provider=provider,
            number_of_queries=queries,
            optional_params=optional_params,
        )
        assert ours == pytest.approx(theirs)


def test_ocr_calls() -> None:
    with closing(SearchOcrServer(OK)) as server:
        ours = ocr(model=OCR_MODEL, document=OCR_DOCUMENT, api_key=GOOD_KEY, api_base=server.url)
        theirs = REFERENCE.ocr(
            model=OCR_MODEL, document=OCR_DOCUMENT, api_key=GOOD_KEY, api_base=server.url
        ).model_dump()
    assert ours.model_dump() == theirs


def test_aocr_calls() -> None:
    with closing(SearchOcrServer(OK)) as server:
        ours = asyncio.run(
            aocr(model=OCR_MODEL, document=OCR_DOCUMENT, api_key=GOOD_KEY, api_base=server.url)
        )
        theirs = asyncio.run(
            REFERENCE_ASYNC.aocr(
                model=OCR_MODEL, document=OCR_DOCUMENT, api_key=GOOD_KEY, api_base=server.url
            )
        ).model_dump()
    assert ours.model_dump() == theirs


def test_ocr_provider_error_class() -> None:
    server = SearchOcrServer(RATE_LIMITED)
    try:
        with pytest.raises(exceptions.RateLimitError) as ours:
            ocr(model=OCR_MODEL, document=OCR_DOCUMENT, api_key=GOOD_KEY, api_base=server.url)
        with pytest.raises(REFERENCE_ERRORS.RateLimitError) as theirs:
            REFERENCE.ocr(
                model=OCR_MODEL, document=OCR_DOCUMENT, api_key=GOOD_KEY, api_base=server.url
            )
    finally:
        server.close()
    assert error_fields(ours.value).status_code == error_fields(theirs.value).status_code


def test_ocr_server_error_class() -> None:
    server = SearchOcrServer(SERVER_ERROR)
    try:
        with pytest.raises(exceptions.InternalServerError) as ours:
            ocr(model=OCR_MODEL, document=OCR_DOCUMENT, api_key=GOOD_KEY, api_base=server.url)
        with pytest.raises(REFERENCE_ERRORS.InternalServerError) as theirs:
            REFERENCE.ocr(
                model=OCR_MODEL, document=OCR_DOCUMENT, api_key=GOOD_KEY, api_base=server.url
            )
    finally:
        server.close()
    assert error_fields(ours.value).status_code == error_fields(theirs.value).status_code


def test_ocr_req_format_error() -> None:
    arguments: Final[dict[str, JsonValue]] = {
        "model": OCR_MODEL,
        "document": OCR_DOCUMENT,
        "api_key": GOOD_KEY,
        "api_base": UNREACHABLE,
        "req_format": "bogus",
        "num_retries": 0,
    }
    with pytest.raises(exceptions.UnsupportedParamsError) as ours:
        ocr(**arguments)
    with pytest.raises(REFERENCE_ERRORS.UnsupportedParamsError) as theirs:
        REFERENCE.ocr(**arguments)
    assert error_fields(theirs.value).status_code == ours.value.status_code
    for value in ("req_format", "bogus", "native", "litellm"):
        assert value in ours.value.message
        assert value in error_fields(theirs.value).message


def test_ocr_document_type_error() -> None:
    document: Final[dict[str, JsonValue]] = {"document_url": "https://example.test/a.pdf"}
    server = SearchOcrServer(OK)
    try:
        with pytest.raises(exceptions.BadRequestError) as ours:
            ocr(model=OCR_MODEL, document=document, api_key=GOOD_KEY, api_base=server.url)
        with pytest.raises(REFERENCE_ERRORS.BadRequestError) as theirs:
            REFERENCE.ocr(
                model=OCR_MODEL, document=document, api_key=GOOD_KEY, api_base=server.url
            )
        calls = list(server.calls)
    finally:
        server.close()
    assert not calls
    assert "type" in str(ours.value)
    assert error_fields(theirs.value).status_code == ours.value.status_code


def test_ocr_flag_unsupported() -> None:
    with pytest.raises(UnsupportedCallArgument) as error:
        ocr(model=OCR_MODEL, document=OCR_DOCUMENT, aocr=True)
    assert error.value.argument == "aocr"


@pytest.mark.parametrize("name", ["client", "shared_session"])
def test_search_unsupported(name: str) -> None:
    blocked: dict[str, JsonValue] = {name: "an object Rust cannot take"}
    with pytest.raises(UnsupportedCallArgument) as error:
        search(query="brave search", search_provider=SEARCH_PROVIDER, **blocked)
    assert error.value.argument == name


@pytest.mark.parametrize("name", ["client", "shared_session"])
def test_ocr_unsupported(name: str) -> None:
    blocked: dict[str, JsonValue] = {name: "an object Rust cannot take"}
    with pytest.raises(UnsupportedCallArgument) as error:
        ocr(model=OCR_MODEL, document=OCR_DOCUMENT, **blocked)
    assert error.value.argument == name
