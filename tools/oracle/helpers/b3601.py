import sys
from types import ModuleType
from typing import Final, Literal, Protocol, TypedDict

from litellm.proxy._types import PassThroughGuardrailSettings, UserAPIKeyAuth
from litellm.proxy.pass_through_endpoints.jsonpath_extractor import JsonPathExtractor
from litellm.proxy.pass_through_endpoints.passthrough_guardrails import (
    PassthroughGuardrailHandler,
)
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
Config = list[str] | dict[str, PassThroughGuardrailSettings | None] | None


class Extractors(Protocol):
    @staticmethod
    def evaluate(data: dict[str, JsonValue], expr: str) -> str | list[str] | None: ...
    @staticmethod
    def extract_fields(data: dict[str, JsonValue], jsonpath_expressions: list[str]) -> str: ...


EXTRACTOR: Extractors = JsonPathExtractor
PROXY_MODULE: Final = "litellm.proxy.proxy_server"


class ProxyServer(ModuleType):
    premium_user: bool = False


class EvaluateCall(TypedDict):
    function: Literal["evaluate"]
    data: dict[str, JsonValue]
    expression: str


class ExtractFieldsCall(TypedDict):
    function: Literal["extract_fields"]
    data: dict[str, JsonValue]
    expressions: list[str]


class NormalizeConfigCall(TypedDict):
    function: Literal["normalize_config"]
    config: Config


class GetSettingsCall(TypedDict):
    function: Literal["get_settings"]
    config: Config
    name: str


class CollectGuardrailsCall(TypedDict):
    function: Literal["collect_guardrails"]
    key_metadata: dict[str, JsonValue]
    team_metadata: dict[str, JsonValue]
    config: Config


Call = (
    EvaluateCall
    | ExtractFieldsCall
    | NormalizeConfigCall
    | GetSettingsCall
    | CollectGuardrailsCall
)


class Input(TypedDict):
    calls: list[Call]


def evaluate_answer(call: EvaluateCall) -> JsonValue:
    value = EXTRACTOR.evaluate(call["data"], call["expression"])
    return JSON.validate_python(value)


def extract_fields_answer(call: ExtractFieldsCall) -> JsonValue:
    value = EXTRACTOR.extract_fields(call["data"], call["expressions"])
    return JSON.validate_python(value)


def normalize_answer(call: NormalizeConfigCall) -> JsonValue:
    normalized = PassthroughGuardrailHandler.normalize_config(call["config"])
    return JSON.validate_python(normalized)


def settings_answer(call: GetSettingsCall) -> JsonValue:
    settings = PassthroughGuardrailHandler.get_settings(call["config"], call["name"])
    if settings is None:
        return None
    return JSON.validate_json(settings.model_dump_json())


def collect_answer(call: CollectGuardrailsCall) -> JsonValue:
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.premium_user = True
    saved = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = proxy_server
    try:
        auth = UserAPIKeyAuth(metadata=call["key_metadata"], team_metadata=call["team_metadata"])
        value = PassthroughGuardrailHandler.collect_guardrails(auth, call["config"])
        return JSON.validate_python(value)
    finally:
        if saved is None:
            sys.modules.pop(PROXY_MODULE, None)
        else:
            sys.modules[PROXY_MODULE] = saved


def answer(call: Call) -> JsonValue:
    if call["function"] == "evaluate":
        return evaluate_answer(call)
    if call["function"] == "extract_fields":
        return extract_fields_answer(call)
    if call["function"] == "normalize_config":
        return normalize_answer(call)
    if call["function"] == "get_settings":
        return settings_answer(call)
    return collect_answer(call)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return answers
