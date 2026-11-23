import os
from collections.abc import Mapping
from dataclasses import dataclass
from typing import NotRequired, TypedDict

import litellm
from litellm.constants import AZURE_OPENAI_AUDIO_PROVIDERS
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.llms.azure.common_utils import get_azure_ad_token
from litellm.llms.azure.realtime.handler import azure_realtime_protocol_for_client
from litellm.llms.base_llm.realtime.transformation import BaseRealtimeConfig
from litellm.llms.xai.common_utils import XAIModelInfo
from litellm.secret_managers.main import get_secret_str
from litellm.types.realtime import RealtimeQueryParams
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter

AZURE_DEFAULT_API_VERSION = "2024-10-01-preview"
OPENAI_API_BASE = "https://api.openai.com/"
XAI_API_BASE = "https://api.x.ai/v1"
PROVIDER_TAGS = {"GeminiRealtimeConfig": "gemini", "MetaRealtimeConfig": "meta"}
AWS_FIELDS = (
    "aws_region_name", "aws_access_key_id", "aws_secret_access_key", "aws_session_token",
    "aws_role_name", "aws_session_name", "aws_profile_name", "aws_web_identity_token",
    "aws_sts_endpoint", "aws_bedrock_runtime_endpoint", "aws_external_id", "aws_session_tags",
)
TEXT = TypeAdapter(str | None)
OBJECT = TypeAdapter(dict[str, JsonValue] | None)
QUERY = TypeAdapter(RealtimeQueryParams)


class Scope(TypedDict):
    headers: list[tuple[bytes, bytes]]


# the oracle's client sends no OpenAI-Beta header, so the scope carries none
class HeaderlessClient:
    @property
    def scope(self) -> Scope:
        return {"headers": []}


class Scenario(TypedDict):
    model: str
    params: NotRequired[dict[str, JsonValue]]
    query_params: NotRequired[dict[str, str]]
    settings: NotRequired[dict[str, JsonValue]]
    token: NotRequired[dict[str, list[str]]]


class Input(TypedDict):
    scenarios: list[Scenario]


@dataclass
class Case:
    model: str
    provider: str
    api_base: str | None
    api_key: str | None
    params: dict[str, JsonValue]
    query: dict[str, str] | None
    tokens: list[str]
    projects: list[str]


def text_field(values: dict[str, JsonValue], name: str) -> str | None:
    value = values.get(name)
    return TEXT.validate_python(value)


def first_text(values: list[str | None]) -> str | None:
    for value in values:
        if value is not None and value != "":
            return value
    return None


def first_json(values: list[JsonValue]) -> JsonValue:
    for value in values:
        if value is None or value in ("", [], {}):
            continue
        return value
    return None


def pairs(values: Mapping[str, JsonValue] | None) -> list[JsonValue]:
    if values is None:
        return []
    return [[name, values[name]] for name in sorted(values)]


def merge_headers(params: dict[str, JsonValue]) -> dict[str, JsonValue]:
    merged: dict[str, JsonValue] = {}
    for name in ("headers", "extra_headers"):
        values = OBJECT.validate_python(params.get(name))
        if values is not None:
            merged.update(values)
    return merged


def build_metadata(params: dict[str, JsonValue]) -> dict[str, JsonValue]:
    internal = OBJECT.validate_python(params.get("litellm_metadata"))
    built: dict[str, JsonValue] = {} if internal is None else dict(internal)
    metadata = OBJECT.validate_python(params.get("metadata"))
    selected: JsonValue = None if metadata is None else metadata.get("guardrails")
    if not selected:
        selected = params.get("guardrails")
    if selected:
        built["guardrails"] = selected
    return built


def put_env(name: str, value: str | None) -> None:
    os.environ.pop(name, None)
    if value is not None:
        os.environ[name] = value


def apply_globals(settings: dict[str, JsonValue]) -> None:
    litellm.api_base = text_field(settings, "api_base")
    litellm.api_key = text_field(settings, "api_key")
    litellm.openai_key = text_field(settings, "openai_key")
    litellm.xai_key = text_field(settings, "xai_key")
    litellm.vertex_project = text_field(settings, "vertex_project")
    litellm.vertex_location = text_field(settings, "vertex_location")
    put_env("AZURE_API_BASE", text_field(settings, "azure_api_base"))
    put_env("AZURE_API_KEY", text_field(settings, "azure_api_key"))
    put_env("OPENAI_API_KEY", text_field(settings, "openai_api_key"))
    put_env("XAI_API_BASE", text_field(settings, "xai_api_base"))
    put_env("XAI_API_KEY", text_field(settings, "xai_api_key"))
    put_env("VERTEXAI_PROJECT", text_field(settings, "vertexai_project"))
    put_env("VERTEXAI_LOCATION", text_field(settings, "vertexai_location"))
    put_env("VERTEXAI_CREDENTIALS", text_field(settings, "vertexai_credentials"))
    put_env("LITELLM_AZURE_REALTIME_PROTOCOL", text_field(settings, "azure_realtime_protocol"))


def record(case: Case, branch: str, details: dict[str, JsonValue]) -> dict[str, JsonValue]:
    return {
        "branch": branch,
        "model": case.model,
        "api_base": None,
        "api_key": None,
        "timeout": case.params.get("timeout"),
        "query": pairs(case.query),
        "metadata": build_metadata(case.params),
        "principal": case.params.get("user_api_key_dict"),
        "headers": None,
        "details": details,
    }


def provider_config(case: Case) -> BaseRealtimeConfig | None:
    try:
        provider = LlmProviders(case.provider)
    except ValueError:
        return None
    return ProviderConfigManager.get_provider_realtime_config(model=case.model, provider=provider)


def dispatch(case: Case) -> dict[str, JsonValue]:
    config = provider_config(case)
    if config is None:
        builder = DIRECT_BRANCHES.get(case.provider)
        if builder is None:
            error = f"Unsupported model: {case.model}"
            return {"branch": "error", "model": None, "error": error}
        return builder(case)
    branch = record(case, PROVIDER_TAGS[type(config).__name__], {})
    branch["api_base"] = text_field(case.params, "api_base")
    branch["api_key"] = text_field(case.params, "api_key")
    branch["headers"] = pairs(merge_headers(case.params))
    return branch


def azure_branch(case: Case) -> dict[str, JsonValue]:
    api_base = first_text([case.api_base, text_field(case.params, "generic_api_base"),
        litellm.api_base, get_secret_str("AZURE_API_BASE")])
    api_key = first_text([
        case.api_key, litellm.api_key, litellm.openai_key, get_secret_str("AZURE_API_KEY")])
    version = first_text([
        text_field(case.params, "api_version"), text_field(case.params, "generic_api_version")])
    if version is None:
        version = AZURE_DEFAULT_API_VERSION
    ad_token = None
    if api_key is None:
        ad_params = TypeAdapter(GenericLiteLLMParams).validate_python(case.params)
        ad_token = get_azure_ad_token(ad_params)
    configured = first_text([text_field(case.params, "realtime_protocol"),
        text_field(case.params, "generic_realtime_protocol"),
        get_secret_str("LITELLM_AZURE_REALTIME_PROTOCOL")])
    protocol = azure_realtime_protocol_for_client(
        configured, query_params=QUERY.validate_python(case.query), websocket=HeaderlessClient()
    )
    details: dict[str, JsonValue] = {
        "api_version": version, "protocol": protocol, "azure_ad_token": ad_token}
    branch = record(case, "azure", details)
    branch["api_base"] = api_base
    branch["api_key"] = api_key
    return branch


def openai_branch(case: Case) -> dict[str, JsonValue]:
    api_base = first_text([
        case.api_base, text_field(case.params, "generic_api_base"), litellm.api_base])
    branch = record(case, "openai", {})
    branch["api_base"] = OPENAI_API_BASE if api_base is None else api_base
    branch["api_key"] = first_text([
        case.api_key, litellm.api_key, litellm.openai_key, get_secret_str("OPENAI_API_KEY")])
    return branch


def xai_branch(case: Case) -> dict[str, JsonValue]:
    api_base = first_text([
        case.api_base, text_field(case.params, "generic_api_base"), get_secret_str("XAI_API_BASE")])
    branch = record(case, "xai", {})
    branch["api_base"] = XAI_API_BASE if api_base is None else api_base
    branch["api_key"] = XAIModelInfo.get_api_key(case.api_key, legacy_generic_before_env=True)
    return branch


def bedrock_branch(case: Case) -> dict[str, JsonValue]:
    details: dict[str, JsonValue] = {name: case.params.get(name) for name in AWS_FIELDS}
    branch = record(case, "bedrock", details)
    branch["api_base"] = first_text([case.api_base, text_field(case.params, "api_base")])
    branch["api_key"] = first_text([case.api_key, text_field(case.params, "api_key")])
    branch["metadata"] = None
    branch["principal"] = None
    branch["query"] = []
    return branch


def vertex_branch(case: Case) -> dict[str, JsonValue]:
    credentials = first_json([
        case.params.get("vertex_credentials"), case.params.get("vertex_ai_credentials"),
        get_secret_str("VERTEXAI_CREDENTIALS")])
    project = first_text([
        text_field(case.params, "vertex_project"), text_field(case.params, "vertex_ai_project"),
        litellm.vertex_project, get_secret_str("VERTEXAI_PROJECT")])
    location = first_text([
        text_field(case.params, "vertex_location"), text_field(case.params, "vertex_ai_location"),
        litellm.vertex_location, get_secret_str("VERTEXAI_LOCATION")])
    if len(case.tokens) == 0:
        raise RuntimeError("the script has no token left")
    resolved_project = "resolved-project"
    if len(case.projects) != 0:
        resolved_project = case.projects[0]
    token_calls: list[JsonValue] = [[credentials, project]]
    details: dict[str, JsonValue] = {
        "access_token": case.tokens[0], "project": resolved_project,
        "location": location, "token_calls": token_calls,
    }
    branch = record(case, "vertex", details)
    branch["api_base"] = first_text([case.api_base, text_field(case.params, "generic_api_base")])
    branch["api_key"] = None
    branch["headers"] = pairs(merge_headers(case.params))
    return branch


DIRECT_BRANCHES = {
    **dict.fromkeys(AZURE_OPENAI_AUDIO_PROVIDERS, azure_branch),
    "openai": openai_branch,
    "bedrock": bedrock_branch,
    "xai": xai_branch,
    "vertex_ai": vertex_branch,
}


def run_scenario(scenario: Scenario) -> JsonValue:
    settings = scenario.get("settings")
    if settings is None:
        settings = {}
    params = scenario.get("params")
    if params is None:
        params = {}
    setup = scenario.get("token")
    tokens = [] if setup is None else setup["tokens"]
    projects = [] if setup is None else setup["projects"]
    apply_globals(settings)
    model, provider, dynamic_api_key, dynamic_api_base = get_llm_provider(
        model=scenario["model"],
        api_base=text_field(params, "api_base"),
        api_key=text_field(params, "api_key"),
    )
    query = scenario.get("query_params")
    if query is not None and "model" in query:
        query = {**query, "model": model}
    case = Case(
        model=model, provider=provider, api_base=dynamic_api_base, api_key=dynamic_api_key,
        params=params, query=query, tokens=tokens, projects=projects,
    )
    return dispatch(case)


def run(argument: Input) -> JsonValue:
    scenarios: list[JsonValue] = [run_scenario(scenario) for scenario in argument["scenarios"]]
    return {"scenarios": scenarios}
