import os
from collections.abc import Awaitable
from dataclasses import dataclass, field
from typing import TypeAlias, TypedDict

import httpx
import litellm
import openai
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.utils import EmbeddingResponse, ModelResponse, TextCompletionResponse
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

EXPERIMENTAL_HANDLER_FLAG = "EXPERIMENTAL_OPENAI_BASE_LLM_HTTP_HANDLER"
KEPT_HEADERS = frozenset({"authorization", "content-type", "openai-organization"})

OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
# mypy reads litellm.completion's decorated return as Any; these name its declared types.
CompletionCall: TypeAlias = ModelResponse | CustomStreamWrapper
TextCompletionCall: TypeAlias = CompletionCall | TextCompletionResponse


class Globals(TypedDict):
    api_base: str | None
    api_key: str | None
    openai_key: str | None
    organization: str | None


class Defaults(TypedDict, total=False):
    max_tokens: int
    n: int
    stop: str | list[str]
    temperature: int
    top_p: int


# Per-case parameters the helper forwards; an embedding's field types must match LiteLLM's
class OptionalParams(TypedDict, total=False):
    dimensions: int
    encoding_format: str
    input: list[str]
    max_tokens: int
    temperature: int


class ProviderCase(TypedDict):
    case: str
    model: str
    api_base: str | None
    api_key: str | None
    organization: str | None
    headers: dict[str, str] | None
    extra_headers: dict[str, str] | None
    metadata: dict[str, JsonValue] | None
    environment: dict[str, str]
    globals: Globals
    preview: bool
    chat_defaults: Defaults | None
    text_defaults: Defaults | None
    messages: list[dict[str, JsonValue]]
    optional_params: OptionalParams
    status: int
    reply: dict[str, JsonValue]
    text_completion: bool


class CallDetails(TypedDict, total=False):
    input: JsonValue
    additional_args: JsonValue
    original_response: JsonValue


CASE: TypeAdapter[ProviderCase] = TypeAdapter(ProviderCase)


def object_arg(value: JsonValue) -> dict[str, JsonValue]:
    match value:
        case dict():
            return OBJECT.validate_python(value)
        case None | bool() | int() | float() | str() | list():
            return {}


def original_kind(value: JsonValue) -> JsonValue:
    if value is None:
        return None
    match value:
        case str():
            return "text"
        case bool() | int() | float() | list() | dict():
            return "object"


@dataclass
class Answer:
    status: int
    reply: dict[str, JsonValue]
    keep: frozenset[str]
    include_headers: bool
    requests: list[JsonValue] = field(default_factory=list)

    def handle(self, request: httpx.Request) -> httpx.Response:
        entry: dict[str, JsonValue] = {
            "method": request.method,
            "url": str(request.url),
            "body": OBJECT.validate_json(request.content),
        }
        if self.include_headers:
            kept = {name: value for name, value in request.headers.items() if name in self.keep}
            entry["headers"] = OBJECT.validate_python(kept)
        self.requests.append(entry)
        return httpx.Response(self.status, json=self.reply, request=request)


class Recorder:
    def __init__(self) -> None:
        self.records: list[JsonValue] = []

    def __call__(self, details: CallDetails) -> None:
        additional = object_arg(details.get("additional_args"))
        if list(additional) != ["headers"]:
            return
        self.records.append(
            {
                "input": details.get("input"),
                "headers": additional["headers"],
                "original_response": original_kind(details.get("original_response")),
            }
        )


def call(
    case: ProviderCase,
    transport: httpx.MockTransport,
    recorder: Recorder,
) -> ModelResponse | TextCompletionResponse | EmbeddingResponse:
    family = case["case"]
    if family == "embedding":
        embedding = litellm.embedding(
            model=case["model"],
            api_base=case["api_base"],
            api_key=case["api_key"],
            headers=case["headers"],
            extra_headers=case["extra_headers"],
            max_retries=0,
            logger_fn=recorder,
            **case["optional_params"],
        )
        return TypeAdapter(EmbeddingResponse).validate_python(embedding)
    if family == "text":
        text: TextCompletionCall | Awaitable[TextCompletionCall] = litellm.completion(
            model=f"text-completion-openai/{case['model']}",
            messages=case["messages"],
            api_base=case["api_base"],
            api_key=case["api_key"],
            headers=case["headers"],
            extra_headers=case["extra_headers"],
            max_retries=0,
            text_completion=case["text_completion"],
            logger_fn=recorder,
            **case["optional_params"],
        )
        return TypeAdapter(ModelResponse | TextCompletionResponse).validate_python(text)
    flag = os.environ.get(EXPERIMENTAL_HANDLER_FLAG)
    if flag is not None and flag.strip().lower() == "true":
        client = HTTPHandler(client=httpx.Client(transport=transport))
        experimental: CompletionCall | Awaitable[CompletionCall] = litellm.completion(
            model=case["model"],
            messages=case["messages"],
            api_base=case["api_base"],
            api_key=case["api_key"],
            headers=case["headers"],
            extra_headers=case["extra_headers"],
            metadata=case["metadata"],
            max_retries=0,
            logger_fn=recorder,
            client=client,
            **case["optional_params"],
        )
        return TypeAdapter(ModelResponse).validate_python(experimental)
    chat: CompletionCall | Awaitable[CompletionCall] = litellm.completion(
        model=case["model"],
        messages=case["messages"],
        api_base=case["api_base"],
        api_key=case["api_key"],
        organization=case["organization"] or case["globals"]["organization"],
        headers=case["headers"],
        extra_headers=case["extra_headers"],
        metadata=case["metadata"],
        max_retries=0,
        logger_fn=recorder,
        **case["optional_params"],
    )
    return TypeAdapter(ModelResponse).validate_python(chat)


def first_object(values: JsonValue) -> dict[str, JsonValue]:
    match values:
        case list() if values:
            return OBJECT.validate_python(values[0])
        case None | bool() | int() | float() | str() | list() | dict():
            return {}


def total_tokens(dump: dict[str, JsonValue]) -> JsonValue:
    usage = dump.get("usage")
    match usage:
        case dict():
            return OBJECT.validate_python(usage).get("total_tokens")
        case None | bool() | int() | float() | str() | list():
            return None


def reply_view(
    case: ProviderCase,
    response: ModelResponse | TextCompletionResponse | EmbeddingResponse,
) -> JsonValue:
    dump = OBJECT.validate_json(response.model_dump_json())
    if case["case"] == "embedding":
        first = first_object(dump.get("data"))
        return {
            "object": dump.get("object"),
            "model": dump.get("model"),
            "embedding": first.get("embedding"),
            "total_tokens": total_tokens(dump),
        }
    first = first_object(dump.get("choices"))
    content: JsonValue = None
    match first.get("message"):
        case dict():
            content = OBJECT.validate_python(first.get("message")).get("content")
        case None | bool() | int() | float() | str() | list():
            content = first.get("text")
    return {
        "id": dump.get("id"),
        "object": dump.get("object"),
        "model": dump.get("model"),
        "content": content,
        "finish_reason": first.get("finish_reason"),
        "total_tokens": total_tokens(dump),
    }


def dispatch_case(case: ProviderCase) -> JsonValue:
    for name, value in case["environment"].items():
        os.environ[name] = value
    globals_case = case["globals"]
    litellm.api_base = globals_case["api_base"]
    litellm.api_key = globals_case["api_key"]
    litellm.openai_key = globals_case["openai_key"]
    litellm.enable_preview_features = case["preview"]
    chat_defaults = case["chat_defaults"]
    if chat_defaults is not None:
        litellm.OpenAIConfig(**chat_defaults)
    text_defaults = case["text_defaults"]
    if text_defaults is not None:
        litellm.OpenAITextCompletionConfig(**text_defaults)
    keep = KEPT_HEADERS
    for mapping in (case["headers"], case["extra_headers"]):
        if mapping is not None:
            keep = keep.union(name.lower() for name in mapping)
    answer = Answer(case["status"], case["reply"], keep, include_headers=case["case"] != "text")
    transport = httpx.MockTransport(answer.handle)
    litellm.client_session = httpx.Client(transport=transport)
    recorder = Recorder()
    try:
        response = call(case, transport, recorder)
    except openai.APIStatusError as error:
        return {
            "requests": answer.requests,
            "reply": None,
            "error": {"status": error.status_code},
            "logs": recorder.records,
        }
    return {
        "requests": answer.requests,
        "reply": reply_view(case, response),
        "error": None,
        "logs": recorder.records,
    }


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = CASE.validate_python(input_value)
    return dispatch_case(case)
