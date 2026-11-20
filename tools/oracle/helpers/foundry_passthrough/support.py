import logging
import os
from collections.abc import Generator, Mapping
from contextlib import contextmanager
from datetime import UTC, datetime
from typing import Final, NotRequired, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from .views import STARTED

Params = dict[str, JsonValue]


class RootCall(TypedDict):
    api_base: str


class PrefixCall(TypedDict):
    root: str
    native_endpoint: str


class QueryCall(TypedDict):
    api_base: str
    deployment_api_version: NotRequired[str | None]
    request_query_params: NotRequired[Params | None]


class BaseCall(TypedDict):
    model: str
    endpoint: str
    api_base: NotRequired[str | None]
    global_api_base: NotRequired[str | None]
    environment_api_base: NotRequired[str | None]
    litellm_params: NotRequired[Params | None]
    request_query_params: NotRequired[Params | None]


class ValidationCall(TypedDict, total=False):
    api_key: str | None
    api_base: str | None
    params: Params | None
    headers: dict[str, str]
    environment: dict[str, str]


class OcrCall(TypedDict, total=False):
    response: str
    malformed: bool


class NonStreamCall(TypedDict):
    status: int
    body: str
    model: str
    endpoint: str
    request_url: NotRequired[str | None]
    ocr: NotRequired[OcrCall | None]


class CollectedCall(TypedDict):
    model: str
    endpoint: str
    chunks: NotRequired[list[str]]
    request_data: NotRequired[Params]


class Input(TypedDict, total=False):
    roots: list[RootCall]
    prefixes: list[PrefixCall]
    queries: list[QueryCall]
    bases: list[BaseCall]
    validations: list[ValidationCall]
    nonstreams: list[NonStreamCall]
    streams: list[CollectedCall]


DEFAULT_URL: Final = "https://res.services.ai.azure.com/providers/mistral/azure/ocr"
START: Final = datetime(2026, 1, 1, tzinfo=UTC)
EMPTY: Final[dict[str, JsonValue]] = {}
AZURE_VARS: Final = (
    "AZURE_AI_API_BASE", "AZURE_AI_API_KEY", "AZURE_AD_TOKEN", "AZURE_TENANT_ID", "AZURE_CLIENT_ID",
    "AZURE_CLIENT_SECRET", "AZURE_USERNAME", "AZURE_PASSWORD", "AZURE_SCOPE",
)
KINDS: Final[dict[str, str]] = {
    "NoneType": "none", "dict": "standard", "OCRResponse": "ocr", "RerankResponse": "relay",
    "ImageResponse": "relay", "EmbeddingResponse": "relay", "ResponseCompletedEvent": "terminal",
    "ResponseIncompleteEvent": "terminal", "ResponseFailedEvent": "terminal",
}

JSON: Final = TypeAdapter(JsonValue)
OBJECT: Final = TypeAdapter(dict[str, JsonValue])


class OcrStub:
    def __init__(self, call: OcrCall) -> None:
        self.result: dict[str, JsonValue] | None = None
        self.malformed: bool = call.get("malformed") is True
        self.calls: list[JsonValue] = []
        text = call.get("response")
        if text is not None:
            self.result = OBJECT.validate_json(text)

    def __call__(self, model: str, endpoint: str, body: bytes) -> Mapping[str, JsonValue] | None:
        call: dict[str, JsonValue] = {
            "model": model,
            "endpoint": endpoint,
            "body": body.decode("utf-8"),
        }
        self.calls.append(call)
        if self.malformed:
            raise ValueError("invalid OCR request: malformed OCR body")
        return self.result


class WarningRecorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


@contextmanager
def azure_env(values: dict[str, str]) -> Generator[None]:
    saved = {name: os.environ.get(name) for name in AZURE_VARS}
    for name in AZURE_VARS:
        os.environ.pop(name, None)
    for name, value in values.items():
        os.environ[name] = value
    try:
        yield
    finally:
        for name in AZURE_VARS:
            os.environ.pop(name, None)
        for name, previous in saved.items():
            if previous is not None:
                os.environ[name] = previous


@contextmanager
def litellm_globals(api_base: str | None, *, refresh_enabled: bool) -> Generator[None]:
    saved_base = litellm.api_base
    saved_refresh = litellm.enable_azure_ad_token_refresh
    litellm.api_base = api_base
    litellm.enable_azure_ad_token_refresh = refresh_enabled
    try:
        yield
    finally:
        litellm.api_base = saved_base
        litellm.enable_azure_ad_token_refresh = saved_refresh


def make_logging(model: str, *, stream: bool) -> Logging:
    return Logging(
        model=model,
        messages=None,
        stream=stream,
        call_type=STARTED,
        start_time=START,
        litellm_call_id="foundry-passthrough-call",
        function_id="foundry-passthrough-function",
    )


def make_response(call: NonStreamCall) -> httpx.Response:
    url = call.get("request_url")
    if url is None:
        url = DEFAULT_URL
    request = httpx.Request("POST", url)
    return httpx.Response(call["status"], content=call["body"].encode(), request=request)


def kind_of(name: str) -> str:
    if name in KINDS:
        return KINDS[name]
    return "unknown"
