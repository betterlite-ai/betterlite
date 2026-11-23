import json
import os
from collections.abc import Callable
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.utils import SearchProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, JsonValue, TypeAdapter

from xai_files import LOG

Params: TypeAlias = dict[str, JsonValue]
Headers: TypeAlias = dict[str, str]
Validate: TypeAlias = Callable[[Headers, str | None, str | None], Headers]
Url: TypeAlias = Callable[[str | None, Params, Params], str]
TransformRequest: TypeAlias = Callable[[str | list[str], Params], object]  # ast-grep-ignore
TransformReply: TypeAlias = Callable[[httpx.Response, Logging], BaseModel]

JSON: Final = TypeAdapter(JsonValue)
PARAMS: Final[TypeAdapter[Params]] = TypeAdapter(Params)
# LiteLLM's class carries Any in its constructor, so the catch names it through a typed alias
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException


class ReplyCall(TypedDict):
    status: int
    body: str


class RequestCall(TypedDict):
    op: Literal["request"]
    query: str | list[str]
    optional_params: Params
    api_key: str | None
    api_base: str | None
    environment: Headers
    headers: Headers
    reply: ReplyCall | None


class RegistryCall(TypedDict):
    op: Literal["registry"]
    providers: list[str]
    environment: Headers


Input: TypeAlias = RequestCall | RegistryCall


class TinyfishConfig(Protocol):
    @property
    def validate_environment(self) -> Validate: ...

    @property
    def get_complete_url(self) -> Url: ...

    @property
    def transform_search_request(self) -> TransformRequest: ...

    @property
    def transform_search_response(self) -> TransformReply: ...


class ErrorRecord(BaseModel):
    status_code: int
    message: str


def config() -> TinyfishConfig:
    found = ProviderConfigManager.get_provider_search_config(SearchProviders("tinyfish"))
    if found is None:
        raise ValueError("no search config is registered for tinyfish")
    return found


def environment(values: Headers) -> None:
    os.environ.pop("TINYFISH_API_KEY", None)
    os.environ.pop("TINYFISH_API_BASE", None)
    for name, value in values.items():
        os.environ[name] = value


def reply_value(search: TinyfishConfig, call: ReplyCall) -> JsonValue:
    response = httpx.Response(
        call["status"],
        content=call["body"].encode(),
        request=httpx.Request("GET", "https://api.search.tinyfish.ai"),
    )
    try:
        parsed = search.transform_search_response(response, LOG)
    except PROVIDER_ERROR as error:
        record = ErrorRecord.model_validate(error, from_attributes=True)
        failure: dict[str, JsonValue] = {"status": record.status_code, "message": record.message}
        body: JsonValue = {"error": failure}
        return JSON.validate_python(body)
    return JSON.validate_json(parsed.model_dump_json())


def request_value(search: TinyfishConfig, call: RequestCall) -> JsonValue:
    environment(call["environment"])
    headers = search.validate_environment(dict(call["headers"]), call["api_key"], call["api_base"])
    body = search.transform_search_request(call["query"], dict(call["optional_params"]))
    params: Params = PARAMS.validate_json(json.dumps(body))
    url = search.get_complete_url(call["api_base"], dict(call["optional_params"]), params)
    reply = None if call["reply"] is None else reply_value(search, call["reply"])
    answer: dict[str, JsonValue] = {
        "headers": JSON.validate_python(headers),
        "params": params,
        "url": url,
        "response": reply,
    }
    return JSON.validate_python(answer)


def registry_value(provider: str) -> JsonValue:
    try:
        found = ProviderConfigManager.get_provider_search_config(SearchProviders(provider))
    except ValueError:
        return None
    if found is None:
        return None
    view: dict[str, JsonValue] = {
        "display": found.ui_friendly_name(),
        "method": found.get_http_method(),
        "rich": found.supports_rich_search_input(),
        "endpoint": found.get_complete_url(None, dict[str, JsonValue](), None),
    }
    return JSON.validate_python(view)


def run(input_value: Input) -> JsonValue:
    if input_value["op"] == "registry":
        environment(input_value["environment"])
        views: list[JsonValue] = [registry_value(provider) for provider in input_value["providers"]]
        answer: dict[str, JsonValue] = {"providers": views}
        return answer
    return request_value(config(), input_value)
