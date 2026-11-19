import json
import os
from collections.abc import Callable
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict

import httpx
from litellm.constants import DEFAULT_DATAFORSEO_LOCATION_CODE as LOCATION_CODE
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.secret_managers.main import get_secret_str
from litellm.types.utils import SearchProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, JsonValue, TypeAdapter

Params: TypeAlias = dict[str, JsonValue]
Headers: TypeAlias = dict[str, str]
Environment: TypeAlias = dict[str, str]
ValidateEnvironment: TypeAlias = Callable[[Headers, str | None, str | None], Headers]
CompleteUrl: TypeAlias = Callable[[str | None, Params], str]
TransformRequest: TypeAlias = Callable[[str | list[str], Params], object]  # ast-grep-ignore
TransformReply: TypeAlias = Callable[[httpx.Response, Logging], BaseModel]

JSON: Final = TypeAdapter(JsonValue)
ENVIRONMENT: Final = (
    "DATAFORSEO_LOGIN",
    "DATAFORSEO_PASSWORD",
    "DATAFORSEO_API_BASE",
)
DEFAULT_API_BASE: Final = "https://api.dataforseo.com/v3/serp/google/organic/live/advanced"


class SettingsCall(TypedDict):
    kind: Literal["settings"]


class ValidateCall(TypedDict):
    kind: Literal["validate_environment"]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]
    headers: NotRequired[Headers]


class UrlCall(TypedDict):
    kind: Literal["complete_url"]
    api_base: NotRequired[str | None]


class RequestCall(TypedDict):
    kind: Literal["request"]
    query: str | list[str]
    optional_params: NotRequired[Params]


class ReplyCall(TypedDict):
    kind: Literal["reply"]
    body: str


Call: TypeAlias = SettingsCall | ValidateCall | UrlCall | RequestCall | ReplyCall


class Input(TypedDict):
    environment: NotRequired[Environment]
    calls: list[Call]


class SearchConfig(Protocol):
    @property
    def validate_environment(self) -> ValidateEnvironment: ...

    @property
    def get_complete_url(self) -> CompleteUrl: ...

    @property
    def transform_search_request(self) -> TransformRequest: ...

    @property
    def transform_search_response(self) -> TransformReply: ...


class LoggingRecord(Logging):
    optional_params: dict[str, JsonValue]

    def __init__(self) -> None:
        self.optional_params = {}


def config() -> SearchConfig:
    found = ProviderConfigManager.get_provider_search_config(SearchProviders.DATAFORSEO)
    if found is None:
        raise ValueError("no search config is registered for dataforseo")
    return found


def apply_environment(environment: Environment) -> None:
    for name in ENVIRONMENT:
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def secret(name: str) -> str | None:
    value = get_secret_str(name)
    if value is None or value == "":
        return None
    return value


def failure() -> dict[str, JsonValue]:
    answer: dict[str, JsonValue] = {"ok": False}
    return answer


def settings_answer() -> JsonValue:
    base = secret("DATAFORSEO_API_BASE")
    answer: dict[str, JsonValue] = {
        "login": secret("DATAFORSEO_LOGIN"),
        "password": secret("DATAFORSEO_PASSWORD"),
        "api_base": DEFAULT_API_BASE if base is None else base,
        "location_code": LOCATION_CODE,
    }
    return answer


def operation_answer(search: SearchConfig, call: Call) -> JsonValue:
    value: JsonValue
    try:
        if call["kind"] == "validate_environment":
            headers = call.get("headers")
            validated = search.validate_environment(
                {} if headers is None else headers,
                call.get("api_key"),
                call.get("api_base"),
            )
            value = JSON.validate_python(validated)
        elif call["kind"] == "complete_url":
            value = search.get_complete_url(call.get("api_base"), {})
        elif call["kind"] == "request":
            params = call.get("optional_params")
            body = search.transform_search_request(
                call["query"], {} if params is None else params
            )
            value = JSON.validate_json(json.dumps(body))
        elif call["kind"] == "settings":
            value = settings_answer()
        else:
            response = httpx.Response(200, content=call["body"].encode())
            parsed = search.transform_search_response(response, LoggingRecord())
            value = JSON.validate_json(parsed.model_dump_json())
    except (TypeError, ValueError):
        return failure()
    answer: dict[str, JsonValue] = {"ok": True, "value": value}
    return answer


def run(input_value: Input) -> JsonValue:
    environment = input_value.get("environment")
    apply_environment({} if environment is None else environment)
    search = config()
    return [operation_answer(search, call) for call in input_value["calls"]]
