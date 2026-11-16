import io
import os
from collections.abc import Callable
from contextlib import redirect_stdout
from datetime import UTC, datetime
from typing import Final, NotRequired, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.exceptions import UnsupportedParamsError
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure_ai.chat.transformation import AzureAIStudioConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse, ProviderField
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JsonObject: TypeAlias = dict[str, JsonValue]
Messages: TypeAlias = list[AllMessageValues]
Headers: TypeAlias = dict[str, str]
AZURE_NAMES: Final[tuple[str, ...]] = (
    "AZURE_AI_API_BASE", "AZURE_AI_API_KEY", "AZURE_OPENAI_API_KEY", "AZURE_API_KEY")
# the exception class carries Any, so the catch names it through a typed alias
UNSUPPORTED: Final[type[Exception]] = UnsupportedParamsError


class UrlCall(TypedDict):
    api_base: str | None
    api_version: str | None


class MapCall(TypedDict):
    model: str
    non_default: JsonObject
    optional: JsonObject
    drop_params: bool
    global_drop_params: bool


class RequestCall(TypedDict):
    model: str
    messages: Messages
    optional: JsonObject


class ResponseCall(TypedDict):
    model: str
    body: str


class ProviderCall(TypedDict):
    model: str
    api_base: str | None
    api_key: str | None
    environment: Headers


class AuthCall(TypedDict):
    api_base: str | None
    api_key: str | None
    headers: Headers
    litellm_params: JsonObject
    environment: Headers


class Input(TypedDict):
    model_cost: dict[str, JsonObject]
    chat_models: list[str]
    text_models: list[str]
    # the Rust adapter fills litellm.open_ai_embedding_models, which no typed assignment reaches
    embedding_models: NotRequired[list[str]]
    urls: NotRequired[list[UrlCall]]
    supported: NotRequired[list[str]]
    maps: NotRequired[list[MapCall]]
    requests: NotRequired[list[RequestCall]]
    responses: NotRequired[list[ResponseCall]]
    providers: NotRequired[list[ProviderCall]]
    auths: NotRequired[list[AuthCall]]
    fields: NotRequired[bool]


class UsageRecord(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class MessageRecord(TypedDict):
    role: str
    content: NotRequired[str | None]


class ChoiceRecord(TypedDict):
    index: int
    message: MessageRecord
    finish_reason: NotRequired[str | None]


class ResponseRecord(TypedDict):
    id: str
    object: str
    created: int
    model: NotRequired[str | None]
    system_fingerprint: NotRequired[str | None]
    usage: NotRequired[UsageRecord | None]
    choices: list[ChoiceRecord]


ReplyArgs: TypeAlias = Callable[
    [
        str, httpx.Response, ModelResponse, Logging, JsonObject, Messages,
        JsonObject, JsonObject, None, str | None, bool | None,
    ],
    ModelResponse,
]


@runtime_checkable
class ChatConfig(Protocol):
    @property
    def get_complete_url(
        self,
    ) -> Callable[[str | None, str | None, str, JsonObject, JsonObject], str]: ...

    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: JsonObject,
        optional_params: JsonObject,
        model: str,
        *,
        drop_params: bool,
    ) -> JsonObject: ...

    def get_required_params(self) -> list[ProviderField]: ...

    @property
    def transform_request(
        self,
    ) -> Callable[[str, Messages, JsonObject, JsonObject, JsonObject], JsonObject]: ...

    @property
    def validate_environment(
        self,
    ) -> Callable[
        [Headers, str, Messages, JsonObject, JsonObject, str | None, str | None], Headers
    ]: ...

    @property
    def transform_response(self) -> ReplyArgs: ...


class Failure(BaseModel):
    status_code: int | None


class ConfigBoundary(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: ChatConfig


CONFIG: Final[ChatConfig] = ConfigBoundary.model_validate({"config": AzureAIStudioConfig()}).config
RESPONSE: Final[TypeAdapter[ResponseRecord]] = TypeAdapter(ResponseRecord)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


def apply_environment(environment: Headers) -> None:
    for name in AZURE_NAMES:
        os.environ.pop(name, None)
    os.environ.update(environment)


def url_value(call: UrlCall) -> JsonValue:
    params: JsonObject = {"api_version": call["api_version"]}
    try:
        url = CONFIG.get_complete_url(call["api_base"], None, "fixture-model", {}, params)
    except ValueError:
        return {"error": {"kind": "ValueError"}}
    return {"value": url}


def map_value(call: MapCall) -> JsonValue:
    litellm.drop_params = call["global_drop_params"]
    optional: JsonObject = dict(call["optional"])
    try:
        mapped = CONFIG.map_openai_params(
            dict(call["non_default"]), optional, call["model"], drop_params=call["drop_params"]
        )
    except UNSUPPORTED as error:
        failure = Failure.model_validate(error, from_attributes=True)
        return {"ok": False, "value": None, "status": failure.status_code}
    return {"ok": True, "value": mapped, "status": None}


def request_value(call: RequestCall) -> JsonValue:
    body = CONFIG.transform_request(call["model"], call["messages"], dict(call["optional"]), {}, {})
    return {"ok": True, "value": body}


def response_value(call: ResponseCall) -> JsonValue:
    logging_obj = Logging(
        model="fixture-model", messages=None, stream=False, call_type="completion",
        start_time=datetime(2026, 1, 1, tzinfo=UTC), litellm_call_id="call-1", function_id="fn-1",
    )
    response = CONFIG.transform_response(
        call["model"],
        httpx.Response(200, content=call["body"].encode()),
        ModelResponse(),
        logging_obj,
        {},
        [],
        {},
        {},
        None,
        None,
        None,
    )
    record = RESPONSE.validate_json(response.model_dump_json())
    usage_record = record.get("usage")
    usage: JsonValue = None if usage_record is None else JSON.validate_python(usage_record)
    choices: list[JsonValue] = [
        {
            "index": choice["index"],
            "message": {
                "role": choice["message"]["role"],
                "content": choice["message"].get("content"),
            },
            "finish_reason": choice.get("finish_reason"),
        }
        for choice in record["choices"]
    ]
    return {
        "id": record["id"],
        "object": record["object"],
        "created": record["created"],
        "model": record.get("model"),
        "system_fingerprint": record.get("system_fingerprint"),
        "usage": usage,
        "choices": choices,
    }


def provider_value(call: ProviderCall) -> JsonValue:
    apply_environment(call["environment"])
    _, provider, key, base = get_llm_provider(
        model=call["model"],
        custom_llm_provider="azure_ai",
        api_base=call["api_base"],
        api_key=call["api_key"],
    )
    return {"provider": provider, "api_base": base, "api_key": key}


def auth_value(call: AuthCall) -> JsonValue:
    apply_environment(call["environment"])
    headers = CONFIG.validate_environment(
        dict(call["headers"]),
        "fixture-model",
        [],
        {},
        call["litellm_params"],
        call["api_key"],
        call["api_base"],
    )
    return JSON.validate_python(headers)


def run(values: Input) -> JsonValue:
    answer: JsonObject = {}
    with redirect_stdout(io.StringIO()):
        litellm.model_cost = values["model_cost"]
        litellm.open_ai_chat_completion_models = set(values["chat_models"])
        litellm.open_ai_text_completion_models = set(values["text_models"])
        if "urls" in values:
            answer["urls"] = [url_value(call) for call in values["urls"]]
        if "supported" in values:
            answer["supported"] = JSON.validate_python(
                [CONFIG.get_supported_openai_params(model) for model in values["supported"]]
            )
        if "maps" in values:
            answer["maps"] = [map_value(call) for call in values["maps"]]
        if "requests" in values:
            answer["requests"] = [request_value(call) for call in values["requests"]]
        if "responses" in values:
            answer["responses"] = [response_value(call) for call in values["responses"]]
        if "providers" in values:
            answer["providers"] = [provider_value(call) for call in values["providers"]]
        if "auths" in values:
            answer["auths"] = [auth_value(call) for call in values["auths"]]
        if values.get("fields") is True:
            answer["fields"] = JSON.validate_python(
                [
                    {
                        "field_name": field["field_name"],
                        "field_type": field["field_type"],
                        "field_description": field["field_description"],
                        "field_value": field["field_value"],
                    }
                    for field in CONFIG.get_required_params()
                ]
            )
    return answer
