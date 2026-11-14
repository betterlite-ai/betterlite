import os
from collections.abc import Callable
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.parallel_ai.search.cost_calculator import PARALLEL_AI_USAGE_PARAM
from litellm.types.utils import SearchProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, JsonValue, TypeAdapter

Params: TypeAlias = dict[str, JsonValue]
Headers: TypeAlias = dict[str, str]
Environment: TypeAlias = dict[str, str]
Provider: TypeAlias = Literal["nimble", "parallel_ai"]
Url: TypeAlias = Callable[[str | None, Params], str]
TransformRequest: TypeAlias = Callable[[str | list[str], Params], JsonValue]
TransformReply: TypeAlias = Callable[[httpx.Response, Logging], BaseModel]
ErrorClass: TypeAlias = Callable[[str, int, Headers], Exception]

JSON: Final = TypeAdapter(JsonValue)
ENVIRONMENT: Final = (
    "NIMBLE_API_KEY",
    "NIMBLE_API_BASE",
    "PARALLEL_AI_API_KEY",
    "PARALLEL_API_KEY",
    "PARALLEL_AI_API_BASE",
)


class UrlCall(TypedDict):
    operation: Literal["url"]
    api_base: NotRequired[str | None]
    environment: NotRequired[Environment]


class RequestCall(TypedDict):
    operation: Literal["request"]
    query: str | list[str]
    optional_params: NotRequired[Params]
    environment: NotRequired[Environment]


class ReplyCall(TypedDict):
    operation: Literal["reply"]
    body: JsonValue
    status: NotRequired[int]
    headers: NotRequired[Headers]
    optional_params: NotRequired[Params]
    environment: NotRequired[Environment]


class ErrorCall(TypedDict):
    operation: Literal["error_class"]
    message: str
    status: int
    headers: NotRequired[Headers]


Call: TypeAlias = UrlCall | RequestCall | ReplyCall | ErrorCall


class Input(TypedDict):
    provider: Provider
    calls: list[Call]


class SearchConfig(Protocol):
    @property
    def get_complete_url(self) -> Url: ...

    @property
    def transform_search_request(self) -> TransformRequest: ...

    @property
    def transform_search_response(self) -> TransformReply: ...

    @property
    def get_error_class(self) -> ErrorClass: ...


class LoggingRecord(Logging):
    optional_params: dict[str, JsonValue]

    def __init__(self, optional_params: dict[str, JsonValue]) -> None:
        self.optional_params = dict(optional_params)


class ErrorRecord(BaseModel):
    status_code: int
    message: str


def config(provider: Provider) -> SearchConfig:
    found = ProviderConfigManager.get_provider_search_config(SearchProviders(provider))
    if found is None:
        raise ValueError(f"no search config is registered for {provider}")
    return found


def apply_environment(environment: Environment) -> None:
    for name in ENVIRONMENT:
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def failure(status: int | None) -> dict[str, JsonValue]:
    answer: dict[str, JsonValue] = {"ok": False}
    if status is not None:
        answer["status"] = status
    return answer


def url_answer(search: SearchConfig, call: UrlCall) -> JsonValue:
    environment = call.get("environment")
    apply_environment({} if environment is None else environment)
    url = search.get_complete_url(call.get("api_base"), {})
    answer: dict[str, JsonValue] = {"ok": True, "value": url}
    return answer


def request_answer(search: SearchConfig, call: RequestCall) -> JsonValue:
    environment = call.get("environment")
    apply_environment({} if environment is None else environment)
    params = call.get("optional_params")
    try:
        body = search.transform_search_request(call["query"], {} if params is None else params)
    except (TypeError, ValueError):
        return failure(None)
    # LiteLLM spreads tuples for the domain lists, which json.dumps writes as arrays
    answer: dict[str, JsonValue] = {"ok": True, "value": body}
    return answer


def reply_answer(search: SearchConfig, provider: Provider, call: ReplyCall) -> JsonValue:
    environment = call.get("environment")
    apply_environment({} if environment is None else environment)
    params = call.get("optional_params")
    headers = call.get("headers")
    status = call.get("status")
    response = httpx.Response(
        200 if status is None else status,
        headers={} if headers is None else headers,
        json=call["body"],
    )
    logging_object = LoggingRecord({} if params is None else params)
    try:
        parsed = search.transform_search_response(response, logging_object)
    except BaseLLMException as error:
        return failure(error.status_code)
    except (TypeError, ValueError):
        return failure(None)
    reply = JSON.validate_json(parsed.model_dump_json())
    if provider == "nimble":
        single: dict[str, JsonValue] = {"ok": True, "value": reply}
        return single
    usage = logging_object.optional_params.get(PARALLEL_AI_USAGE_PARAM)
    combined: dict[str, JsonValue] = {"reply": reply, "usage": usage}
    answer: dict[str, JsonValue] = {"ok": True, "value": combined}
    return answer


def error_answer(search: SearchConfig, call: ErrorCall) -> JsonValue:
    headers = call.get("headers")
    raised = search.get_error_class(
        call["message"], call["status"], {} if headers is None else headers
    )
    record = ErrorRecord.model_validate(raised, from_attributes=True)
    answer: dict[str, JsonValue] = {
        "status_code": record.status_code,
        "message": record.message,
    }
    return answer


def run(input_value: Input) -> JsonValue:
    provider = input_value["provider"]
    search = config(provider)
    answers: list[JsonValue] = []
    for call in input_value["calls"]:
        if call["operation"] == "url":
            answers.append(url_answer(search, call))
        elif call["operation"] == "request":
            answers.append(request_answer(search, call))
        elif call["operation"] == "reply":
            answers.append(reply_answer(search, provider, call))
        else:
            answers.append(error_answer(search, call))
    return answers
