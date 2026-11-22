import os
from collections.abc import Awaitable
from dataclasses import dataclass, field
from typing import TypeAlias, TypedDict

import httpx
import litellm
import openai
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.types.utils import EmbeddingResponse, ModelResponse
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
# litellm.completion's decorator hides its declared return from mypy; this names the declared type.
CompletionCall: TypeAlias = ModelResponse | CustomStreamWrapper


class Globals(TypedDict):
    api_base: str | None
    api_version: str | None
    api_key: str | None
    azure_key: str | None
    headers: dict[str, str] | None
    azure_default_api_version: str | None


class AzureDefaults(TypedDict, total=False):
    max_tokens: int
    n: int
    stop: str
    temperature: int
    top_p: int


class OSeriesDefaults(TypedDict, total=False):
    max_tokens: int
    temperature: int
    top_p: int


class OptionalParams(TypedDict, total=False):
    max_completion_tokens: int
    max_tokens: int
    n: int
    temperature: int


class EmbeddingBody(TypedDict):
    input: list[str]
    dimensions: int | None
    encoding_format: str | None


class Call(TypedDict):
    case: str
    model: str
    api_base: str | None
    api_version: str | None
    api_key: str | None
    azure_ad_token: str | None
    headers: dict[str, str] | None
    extra_headers: dict[str, str] | None
    extra_body: dict[str, JsonValue] | None
    max_retries: int | bool | None
    messages: list[dict[str, JsonValue]]
    optional_params: OptionalParams
    embedding: EmbeddingBody | None
    acompletion: bool


class ProviderCase(Call):
    environment: dict[str, str]
    globals: Globals
    chat_defaults: AzureDefaults | None
    o_series_defaults: OSeriesDefaults | None
    status: int
    reply: dict[str, JsonValue]
    second: Call | None


@dataclass
class Answer:
    status: int
    reply: dict[str, JsonValue]
    keep: frozenset[str]
    requests: list[JsonValue] = field(default_factory=list)

    def handle(self, request: httpx.Request) -> httpx.Response:
        kept = {name: value for name, value in request.headers.items() if name in self.keep}
        self.requests.append(
            {
                "method": request.method,
                "url": str(request.url),
                "headers": OBJECT.validate_python(kept),
                "body": OBJECT.validate_json(request.content),
            }
        )
        return httpx.Response(self.status, json=self.reply, request=request)


def call(step: Call) -> tuple[str, ModelResponse | EmbeddingResponse]:
    family = step["case"]
    if family == "embedding":
        body = step["embedding"]
        if body is None:
            raise ValueError("an embedding case names its body")
        embedded = litellm.embedding(
            model=f"azure/{step['model']}",
            input=body["input"],
            dimensions=body["dimensions"],
            encoding_format=body["encoding_format"],
            api_base=step["api_base"],
            api_version=step["api_version"],
            api_key=step["api_key"],
            azure_ad_token=step["azure_ad_token"],
            headers=step["headers"],
            extra_headers=step["extra_headers"],
            extra_body=step["extra_body"],
            max_retries=0,
        )
        return (family, TypeAdapter(EmbeddingResponse).validate_python(embedded))
    provider = "azure_text" if family == "text" else "azure"
    response: CompletionCall | Awaitable[CompletionCall] = litellm.completion(
        model=f"{provider}/{step['model']}",
        messages=step["messages"],
        api_base=step["api_base"],
        api_version=step["api_version"],
        api_key=step["api_key"],
        azure_ad_token=step["azure_ad_token"],
        headers=step["headers"],
        extra_headers=step["extra_headers"],
        extra_body=step["extra_body"],
        max_retries=step["max_retries"],
        **step["optional_params"],
    )
    return (family, TypeAdapter(ModelResponse).validate_python(response))


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


def reply_view(family: str, response: ModelResponse | EmbeddingResponse) -> dict[str, JsonValue]:
    dump = OBJECT.validate_json(response.model_dump_json())
    if family == "embedding":
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


def kept_headers(case: ProviderCase) -> frozenset[str]:
    names = {"authorization", "content-type", "api-key"}
    for step in (case, case["second"]):
        if step is not None:
            for mapping in (step["headers"], step["extra_headers"]):
                if mapping is not None:
                    names.update(name.lower() for name in mapping)
    return frozenset(names)


def dispatch_case(case: ProviderCase) -> JsonValue:
    for name, value in case["environment"].items():
        os.environ[name] = value
    module = case["globals"]
    litellm.api_base = module["api_base"]
    litellm.api_version = module["api_version"]
    litellm.api_key = module["api_key"]
    litellm.azure_key = module["azure_key"]
    litellm.AZURE_DEFAULT_API_VERSION = module["azure_default_api_version"] or "2025-02-01-preview"
    chat_defaults = case["chat_defaults"]
    if chat_defaults is not None:
        litellm.AzureOpenAIConfig(**chat_defaults)
    o_series_defaults = case["o_series_defaults"]
    if o_series_defaults is not None:
        litellm.AzureOpenAIO1Config(**o_series_defaults)
    answer = Answer(case["status"], case["reply"], kept_headers(case))
    transport = httpx.MockTransport(answer.handle)
    litellm.client_session = httpx.Client(transport=transport)
    replies: list[JsonValue] = []
    error: JsonValue = None
    for step in (case, case["second"]):
        if step is None:
            continue
        try:
            family, response = call(step)
        except openai.APIStatusError as failure:
            error = {"status": failure.status_code}
            break
        except (ValueError, openai.OpenAIError):
            error = {"status": 500}
            break
        replies.append(reply_view(family, response))
    return {"requests": answer.requests, "replies": replies, "error": error}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    return dispatch_case(TypeAdapter(ProviderCase).validate_python(input_value))
