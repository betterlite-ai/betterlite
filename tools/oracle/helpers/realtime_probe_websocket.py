import os
import ssl
from collections.abc import Mapping
from typing import TypedDict

import litellm
from litellm.constants import REALTIME_WEBSOCKET_MAX_MESSAGE_SIZE_BYTES as MAX_SIZE
from litellm.llms.azure.common_utils import get_azure_ad_token
from litellm.llms.azure.realtime.handler import AzureOpenAIRealtime
from litellm.llms.custom_httpx.http_handler import get_shared_realtime_ssl_context
from litellm.llms.vertex_ai.audio_transcription.realtime_transformation import (
    is_vertex_speech_to_text_model,
)
from litellm.llms.vertex_ai.realtime.transformation import VertexAIRealtimeConfig
from litellm.llms.vertex_ai.vertex_llm_base import VertexBase
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue, TypeAdapter

AZURE_DEFAULT_API_VERSION = "2024-10-01-preview"
OPENAI_API_BASE = "https://api.openai.com/"
XAI_API_BASE = "https://api.x.ai/v1"
DEFAULT_PROJECT = "resolved-project"
GA_PROTOCOL = "GA"


class StoredCredential(TypedDict):
    credential_name: str
    credential_values: dict[str, JsonValue]


class Settings(TypedDict, total=False):
    azure_realtime_protocol: str | None
    credential_timeout_seconds: float
    max_message_size_bytes: int


class RequiredScenario(TypedDict):
    model: str
    custom_llm_provider: str


class Scenario(RequiredScenario, total=False):
    api_key: str | None
    api_base: str | None
    api_version: str | None
    realtime_protocol: str | None
    model_params: dict[str, JsonValue]
    stored: list[StoredCredential]
    azure_ad_token: str | None
    vertex_token: str | None
    vertex_project: str | None
    vertex_location: str | None
    settings: Settings


class Input(TypedDict):
    scenarios: list[Scenario]


def empty_view(branch: str) -> dict[str, JsonValue]:
    return {
        "branch": branch,
        "api_base": None,
        "api_version": None,
        "protocol": None,
        "url": None,
        "headers": [],
        "max_size": None,
        "ssl": None,
    }


def text_field(values: Mapping[str, JsonValue], name: str) -> str | None:
    match values.get(name):
        case str() as text:
            return text
        case None | bool() | int() | float() | list() | dict():
            return None


# _model_params_with_stored_credentials: stored values sit underneath the explicit parameters
def merged_parameters(scenario: Scenario) -> dict[str, JsonValue]:
    params = scenario.get("model_params")
    explicit = {} if params is None else params
    stored = scenario.get("stored")
    entries = [] if stored is None else stored
    name = text_field(explicit, "litellm_credential_name")
    stored_values: dict[str, JsonValue] = {}
    if name is not None:
        for entry in entries:
            if entry["credential_name"] == name:
                stored_values = entry["credential_values"]
                break
    return {**stored_values, **explicit}


# _azure_realtime_health_protocol: the first truthy configured protocol, else GA
def azure_protocol(scenario: Scenario, merged: Mapping[str, JsonValue]) -> str:
    configured: JsonValue = scenario.get("realtime_protocol")
    if not configured:
        configured = merged.get("realtime_protocol")
    if not configured:
        configured = os.environ.get("LITELLM_AZURE_REALTIME_PROTOCOL")
    match configured:
        case str() as text:
            return text
        case None | bool() | int() | float() | list() | dict():
            return GA_PROTOCOL


# `None if api_key else get_azure_ad_token(...)`, as _realtime_health_check_auth_headers reads it
def azure_ad_token(key: str | None, merged: Mapping[str, JsonValue]) -> str | None:
    if key:
        return None
    params = TypeAdapter(GenericLiteLLMParams).validate_python(merged)
    return get_azure_ad_token(params)


def ssl_kind(*, value: bool | str | ssl.SSLContext | None) -> str:
    if value is None:
        return "disabled"
    return "insecure" if value is False else "shared"


def bearer_headers(key: str | None) -> list[JsonValue]:
    pairs: list[JsonValue] = []
    if key is not None:
        pairs.append(["Authorization", f"Bearer {key}"])
    return pairs


def failure_view(branch: str, base: str | None) -> JsonValue:
    view = empty_view(branch)
    view["api_base"] = base
    return view


def azure_answer(
    scenario: Scenario,
    merged: dict[str, JsonValue],
    key: str | None,
    base: str | None,
    version: str | None,
) -> JsonValue:
    token = azure_ad_token(key, merged)
    headers = AzureOpenAIRealtime.get_auth_headers(api_key=key, azure_ad_token=token)
    view = empty_view("azure")
    view["api_base"] = base
    if base is None:
        view["api_base"] = ""
    view["api_version"] = version or AZURE_DEFAULT_API_VERSION
    view["protocol"] = azure_protocol(scenario, merged)
    pairs: list[JsonValue] = [[name, headers[name]] for name in sorted(headers)]
    view["headers"] = pairs
    view["max_size"] = MAX_SIZE
    view["ssl"] = ssl_kind(value=get_shared_realtime_ssl_context())
    return view


def base_answer(branch: str, key: str | None, base: str | None, default: str) -> JsonValue:
    view = empty_view(branch)
    view["api_base"] = base or default
    view["headers"] = bearer_headers(key)
    view["max_size"] = MAX_SIZE
    view["ssl"] = ssl_kind(value=get_shared_realtime_ssl_context())
    return view


def vertex_answer(scenario: Scenario, merged: dict[str, JsonValue], base: str | None) -> JsonValue:
    model = scenario["model"]
    if is_vertex_speech_to_text_model(model):
        return failure_view("speech_to_text", base)
    location = VertexBase.safe_get_vertex_ai_location(dict(merged))
    region = VertexBase.get_vertex_region(vertex_region=location, model=model)
    project = scenario.get("vertex_project") or DEFAULT_PROJECT
    token = scenario.get("vertex_token")
    if token is None:
        token = ""
    config = VertexAIRealtimeConfig(access_token=token, project=project, location=region)
    headers: dict[str, str] = config.validate_environment(headers={}, model=model, api_key=None)  # type: ignore[misc]  # pyright: ignore[reportUnknownVariableType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/realtime/transformation.py#L78-L83
    view = empty_view("vertex_ai")
    view["api_base"] = base
    view["url"] = config.get_complete_url(api_base=base, model=model)
    pairs: list[JsonValue] = [[name, headers[name]] for name in sorted(headers)]
    view["headers"] = pairs
    view["max_size"] = MAX_SIZE
    view["ssl"] = ssl_kind(value=get_shared_realtime_ssl_context())
    return view


def run_scenario(scenario: Scenario) -> JsonValue:
    litellm.vertex_location = scenario.get("vertex_location")
    merged = merged_parameters(scenario)
    key = scenario.get("api_key") or text_field(merged, "api_key")
    base = scenario.get("api_base") or text_field(merged, "api_base")
    version = scenario.get("api_version") or text_field(merged, "api_version")
    provider = scenario["custom_llm_provider"]
    if provider == "azure":
        return azure_answer(scenario, merged, key, base, version)
    if provider == "openai":
        return base_answer("openai", key, base, OPENAI_API_BASE)
    if provider == "xai":
        return base_answer("xai", key, base, XAI_API_BASE)
    if provider == "vertex_ai":
        return vertex_answer(scenario, merged, base)
    return failure_view("unsupported", base)


def run(argument: Input) -> JsonValue:
    scenarios = argument["scenarios"]
    answered = [run_scenario(scenario) for scenario in scenarios]
    return {"scenarios": answered}
