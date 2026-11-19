import logging
import os
from collections.abc import Generator, Mapping
from contextlib import contextmanager
from datetime import UTC, datetime
from typing import Final, NotRequired, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure_ai.passthrough.transformation import (
    AzureAIPassthroughConfig,
    foundry_root,
    relay_query_params,
    without_repeated_native_prefix,
)
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

Params = dict[str, JsonValue]
STARTED: Final = "allm_passthrough_route"
DEFAULT_URL: Final = "https://res.services.ai.azure.com/providers/mistral/azure/ocr"
START: Final = datetime(2026, 1, 1, tzinfo=UTC)
EMPTY: Final[dict[str, JsonValue]] = {}
AZURE_VARS: Final = (
    "AZURE_AI_API_BASE",
    "AZURE_AI_API_KEY",
    "AZURE_AD_TOKEN",
    "AZURE_TENANT_ID",
    "AZURE_CLIENT_ID",
    "AZURE_CLIENT_SECRET",
    "AZURE_USERNAME",
    "AZURE_PASSWORD",
    "AZURE_SCOPE",
)
KINDS: Final[dict[str, str]] = {
    "NoneType": "none",
    "dict": "standard",
    "OCRResponse": "ocr",
    "RerankResponse": "relay",
    "ImageResponse": "relay",
    "EmbeddingResponse": "relay",
    "ResponseCompletedEvent": "terminal",
    "ResponseIncompleteEvent": "terminal",
    "ResponseFailedEvent": "terminal",
}

JSON: Final = TypeAdapter(JsonValue)
OBJECT: Final = TypeAdapter(dict[str, JsonValue])


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


class PageView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    index: int
    markdown: str


class UsageView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    pages_processed: int | None = None
    credits: float | None = None
    doc_size_bytes: int | None = None


class OcrView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    pages: list[PageView]
    model: str
    usage_info: UsageView | None = None


class StandardView(BaseModel):
    response: JsonValue


class LoggingView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    call_type: str


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


CONFIG: Final = AzureAIPassthroughConfig()


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
def litellm_globals(api_base: str | None, refresh_enabled: bool) -> Generator[None]:
    saved_base = litellm.api_base
    saved_refresh = litellm.enable_azure_ad_token_refresh
    litellm.api_base = api_base
    litellm.enable_azure_ad_token_refresh = refresh_enabled
    try:
        yield
    finally:
        litellm.api_base = saved_base
        litellm.enable_azure_ad_token_refresh = saved_refresh


def make_logging(model: str, stream: bool) -> Logging:
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


def call_type_of(logging_obj: Logging) -> str | None:
    call_type = LoggingView.model_validate(logging_obj).call_type
    return None if call_type == STARTED else call_type


def kind_of(name: str) -> str:
    if name in KINDS:
        return KINDS[name]
    return "unknown"


def root_outcome(call: RootCall) -> JsonValue:
    try:
        return {"root": foundry_root(call["api_base"])}
    except (ValueError, httpx.InvalidURL):
        return {"error": {"status": None}}


def prefix_outcome(call: PrefixCall) -> JsonValue:
    try:
        return {"root": without_repeated_native_prefix(call["root"], call["native_endpoint"])}
    except (ValueError, httpx.InvalidURL):
        return {"error": {"status": None}}


def query_outcome(call: QueryCall) -> JsonValue:
    try:
        params = relay_query_params(
            call.get("request_query_params"),
            call.get("deployment_api_version"),
            call["api_base"],
        )
    except (ValueError, httpx.InvalidURL):
        return {"error": {"status": None}}
    if params is None:
        return {"params": None}
    return {"params": JSON.validate_python(dict(params))}


def base_outcome(call: BaseCall) -> JsonValue:
    values: dict[str, str] = {}
    env_base = call.get("environment_api_base")
    if env_base is not None:
        values["AZURE_AI_API_BASE"] = env_base
    params = call.get("litellm_params")
    if params is None:
        params = {}
    with azure_env(values), litellm_globals(call.get("global_api_base"), True):
        try:
            url, root = CONFIG.get_complete_url(
                api_base=call.get("api_base"),
                api_key=None,
                model=call["model"],
                endpoint=call["endpoint"],
                request_query_params=call.get("request_query_params"),
                litellm_params=params,
            )
        except (ValueError, httpx.InvalidURL):
            return {"error": {"status": None}}
        return {"url": str(url), "root": root}


def validation_outcome(call: ValidationCall) -> JsonValue:
    values = call.get("environment")
    if values is None:
        values = {}
    params = call.get("params")
    if params is None:
        params = {}
    headers = call.get("headers")
    if headers is None:
        headers = {}
    with azure_env(values), litellm_globals(None, False):
        try:
            answer = CONFIG.validate_environment(
                headers=headers,
                model="",
                messages=[],
                optional_params=EMPTY,
                litellm_params=params,
                api_key=call.get("api_key"),
                api_base=call.get("api_base"),
            )
        except (TypeError, ValueError):
            return {"error": {"status": None}}
        return {"headers": JSON.validate_python(answer)}


def nonstream_outcome(call: NonStreamCall) -> JsonValue:
    ocr_call = call.get("ocr")
    stub: OcrStub | None = None
    config = CONFIG
    if ocr_call is not None:
        stub = OcrStub(ocr_call)
        config = AzureAIPassthroughConfig(passthrough_ocr=stub)
    logging_obj = make_logging(call["model"], False)
    response = make_response(call)
    recorder = WarningRecorder()
    logger = logging.getLogger("LiteLLM")
    level = logger.level
    logger.setLevel(logging.WARNING)
    logger.addHandler(recorder)
    try:
        answer = config.logging_non_streaming_response(
            model=call["model"],
            custom_llm_provider="azure_ai",
            httpx_response=response,
            request_data=EMPTY,
            logging_obj=logging_obj,
            endpoint=call["endpoint"],
        )
    finally:
        logger.removeHandler(recorder)
        logger.setLevel(level)
    kind = kind_of(type(answer).__name__)
    result: dict[str, JsonValue] = {
        "kind": kind,
        "call_type": call_type_of(logging_obj),
        "ocr_calls": [] if stub is None else stub.calls,
        "warned": bool(recorder.messages),
    }
    if kind == "standard":
        result["standard"] = StandardView.model_validate(answer).response
    if kind == "ocr":
        view = OcrView.model_validate(answer)
        usage = view.usage_info
        usage_value: JsonValue = None
        if usage is not None:
            usage_value = {
                "pages_processed": usage.pages_processed,
                "credits": usage.credits,
                "doc_size_bytes": usage.doc_size_bytes,
            }
        result["ocr"] = {"model": view.model, "pages": len(view.pages), "usage": usage_value}
    return result


def collected_outcome(call: CollectedCall) -> JsonValue:
    chunks = call.get("chunks")
    if chunks is None:
        chunks = []
    request_data = call.get("request_data")
    if request_data is None:
        request_data = {}
    logging_obj = make_logging(call["model"], True)
    answer = CONFIG.handle_logging_collected_chunks(
        all_chunks=chunks,
        litellm_logging_obj=logging_obj,
        model=call["model"],
        custom_llm_provider="azure_ai",
        endpoint=call["endpoint"],
    )
    kind = kind_of(type(answer).__name__)
    streaming = CONFIG.is_streaming_request(call["endpoint"], request_data)
    return {"kind": kind, "call_type": call_type_of(logging_obj), "streaming": streaming}


def run(input_value: Input) -> JsonValue:
    roots = input_value.get("roots")
    prefixes = input_value.get("prefixes")
    queries = input_value.get("queries")
    bases = input_value.get("bases")
    validations = input_value.get("validations")
    nonstreams = input_value.get("nonstreams")
    streams = input_value.get("streams")
    return {
        "roots": [root_outcome(call) for call in roots] if roots is not None else [],
        "prefixes": [prefix_outcome(call) for call in prefixes] if prefixes is not None else [],
        "queries": [query_outcome(call) for call in queries] if queries is not None else [],
        "bases": [base_outcome(call) for call in bases] if bases is not None else [],
        "validations": (
            [validation_outcome(call) for call in validations] if validations is not None else []
        ),
        "nonstreams": (
            [nonstream_outcome(call) for call in nonstreams] if nonstreams is not None else []
        ),
        "streams": [collected_outcome(call) for call in streams] if streams is not None else [],
    }
