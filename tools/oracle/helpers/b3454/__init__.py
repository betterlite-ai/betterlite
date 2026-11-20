import asyncio
import base64
import os
from functools import partial
from typing import Final, NotRequired, TypedDict

import httpx
from aiohttp import ClientSession
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.rag.ingestion import gemini_ingestion
from litellm.rag.ingestion.gemini_ingestion import GeminiRAGIngestion
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

API_KEY_HEADER: Final[str] = "x-goog-api-key"
MASKED_KEY: Final[str] = "<present>"
HEADERS: Final[tuple[str, ...]] = (
    "content-type",
    "x-goog-api-key",
    "x-goog-upload-protocol",
    "x-goog-upload-command",
    "x-goog-upload-header-content-length",
    "x-goog-upload-header-content-type",
    "x-goog-upload-offset",
)
JSON: Final = TypeAdapter(JsonValue)


class Options(TypedDict, total=False):
    name: str
    chunking_strategy: dict[str, JsonValue]
    vector_store: dict[str, JsonValue]


class Step(TypedDict, total=False):
    status: int
    headers: dict[str, str]
    body: str
    failure: str


class Input(TypedDict):
    script: list[Step]
    options: NotRequired[Options]
    file_content_base64: NotRequired[str]
    filename: NotRequired[str]
    content_type: NotRequired[str]
    environment: NotRequired[dict[str, str]]


class Projection(TypedDict):
    method: str
    url: str
    headers: dict[str, str]
    body: JsonValue | None
    raw_body_bytes: int | None
    timeout_seconds: int | None


class StoreView(TypedDict):
    requests: list[Projection]
    vector_store_id: str | None
    file_id: str | None


class ScriptedFailureError(Exception):
    pass


def parsed_body(request: httpx.Request) -> JsonValue | None:
    try:
        decoded: JsonValue = JSON.validate_json(request.content)
    except ValidationError:
        return None
    return decoded


def record_request(request: httpx.Request, timeout: float | None) -> Projection:
    headers: dict[str, str] = {}
    for name in HEADERS:
        values = request.headers.get_list(name)
        if not values:
            continue
        headers[name] = MASKED_KEY if name == API_KEY_HEADER else values[0]
    seconds = None if timeout is None else int(timeout)
    decoded = parsed_body(request)
    raw_bytes = None if decoded is not None else len(request.content)
    return Projection(
        method=request.method,
        url=str(request.url),
        headers=headers,
        body=decoded,
        raw_body_bytes=raw_bytes,
        timeout_seconds=seconds,
    )


class Recorder:
    def __init__(self, script: list[Step]) -> None:
        self._script = script
        self._next = 0
        self.requests: list[Projection] = []

    def answer(self, request: httpx.Request, timeout: float | None) -> httpx.Response:
        self.requests.append(record_request(request, timeout))
        step = self._script[self._next]
        self._next += 1
        failure = step.get("failure")
        if failure is not None:
            raise ScriptedFailureError(failure)
        body = step.get("body")
        content = b"" if body is None else body.encode()
        status = step.get("status")
        if status is None:
            raise ScriptedFailureError("script step carries no status")
        return httpx.Response(
            status, headers=step.get("headers"), content=content, request=request
        )


class ScriptedTransport(httpx.AsyncBaseTransport):
    def __init__(self, recorder: Recorder, timeout: float | None) -> None:
        self._recorder = recorder
        self._timeout = timeout

    @override
    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        return self._recorder.answer(request, self._timeout)


def client_factory(
    recorder: Recorder,
    llm_provider: str,
    params: dict[str, float] | None = None,
    shared_session: ClientSession | None = None,
) -> AsyncHTTPHandler:
    del llm_provider, shared_session
    timeout = None if params is None else params["timeout"]
    return AsyncHTTPHandler(transport=ScriptedTransport(recorder, timeout), timeout=timeout)


def build(options: Options | None) -> GeminiRAGIngestion:
    ingestion = GeminiRAGIngestion({})
    if options is None:
        return ingestion
    vector = options.get("vector_store")
    if vector is not None:
        ingestion.vector_store_config = vector
    strategy = options.get("chunking_strategy")
    if strategy is not None:
        ingestion.chunking_strategy = strategy
    name = options.get("name")
    if name is not None:
        ingestion.ingest_name = name
    return ingestion


def store(input_value: Input, recorder: Recorder) -> StoreView:
    ingestion = build(input_value.get("options"))
    content: bytes | None = None
    encoded = input_value.get("file_content_base64")
    if encoded is not None:
        content = base64.b64decode(encoded)
    vector_store_id, file_id = asyncio.run(
        ingestion.store(
            file_content=content,
            filename=input_value.get("filename"),
            content_type=input_value.get("content_type"),
            chunks=[],
            embeddings=None,
            existing_file_id=None,
        )
    )
    return StoreView(
        requests=recorder.requests, vector_store_id=vector_store_id, file_id=file_id
    )


def run(input_value: Input) -> StoreView:
    recorder = Recorder(input_value["script"])
    gemini_ingestion.get_async_httpx_client = partial(client_factory, recorder)  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rag/ingestion/gemini_ingestion.py#L13-L17
    saved = dict(os.environ)
    os.environ.clear()
    environment = input_value.get("environment")
    if environment is not None:
        os.environ.update(environment)
    try:
        return store(input_value, recorder)
    finally:
        os.environ.clear()
        os.environ.update(saved)
