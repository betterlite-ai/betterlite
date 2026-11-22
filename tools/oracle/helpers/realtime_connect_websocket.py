import os
from dataclasses import dataclass, field
from typing import NotRequired, TypedDict

import litellm
from litellm.llms.azure.realtime.handler import azure_realtime_protocol_for_client
from litellm.llms.xai.common_utils import XAIModelInfo
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.secret_managers.main import get_secret_str
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter

JSON = TypeAdapter(JsonValue)
AZURE_DEFAULT_API_VERSION = "2024-10-01-preview"
OPENAI_DEFAULT_API_BASE = "https://api.openai.com/"
XAI_DEFAULT_API_BASE = "https://api.x.ai/v1"
AWS_FIELDS = (
    "aws_region_name",
    "aws_access_key_id",
    "aws_secret_access_key",
    "aws_session_token",
    "aws_role_name",
    "aws_session_name",
    "aws_profile_name",
    "aws_web_identity_token",
    "aws_sts_endpoint",
    "aws_bedrock_runtime_endpoint",
    "aws_external_id",
    "aws_session_tags",
)


class Params(TypedDict, total=False):
    api_base: str
    api_key: str
    api_version: str
    azure_ad_token: str
    timeout: float
    generic_api_base: str
    generic_api_version: str
    generic_realtime_protocol: str
    headers: dict[str, JsonValue]
    extra_headers: dict[str, JsonValue]
    user: str
    metadata: dict[str, JsonValue]
    litellm_metadata: dict[str, JsonValue]
    guardrails: JsonValue
    user_api_key_dict: dict[str, JsonValue]
    realtime_protocol: str
    vertex_credentials: JsonValue
    vertex_ai_credentials: JsonValue
    vertex_project: str
    vertex_ai_project: str
    vertex_location: str
    vertex_ai_location: str
    tenant_id: str
    aws_region_name: str
    aws_access_key_id: str
    aws_secret_access_key: str
    aws_session_token: str
    aws_role_name: str
    aws_session_name: str
    aws_profile_name: str
    aws_web_identity_token: str
    aws_sts_endpoint: str
    aws_bedrock_runtime_endpoint: str
    aws_external_id: str
    aws_session_tags: JsonValue


class Settings(TypedDict, total=False):
    api_base: str
    api_key: str
    openai_key: str
    xai_key: str
    vertex_project: str
    vertex_location: str
    azure_api_base: str
    azure_api_key: str
    openai_api_key: str
    xai_api_base: str
    xai_api_key: str
    vertexai_credentials: str
    vertexai_project: str
    vertexai_location: str
    azure_realtime_protocol: str


class TokenSetup(TypedDict, total=False):
    tokens: list[str]
    projects: list[str]
    pending: bool


class Scenario(TypedDict):
    model: str
    params: NotRequired[Params]
    query_params: NotRequired[dict[str, str]]
    client_sent_beta: NotRequired[bool]
    settings: NotRequired[Settings]
    token: NotRequired[TokenSetup]
    ad_token: NotRequired[str]


class Input(TypedDict):
    scenarios: list[Scenario]


@dataclass
class Resolved:
    model: str
    provider: str
    api_base: str | None
    api_key: str | None


@dataclass
class CallInputs:
    params: Params
    settings: Settings
    resolved: Resolved
    query: dict[str, str] | None
    beta: bool
    script: TokenScript


@dataclass
class Branch:
    name: str
    model: str | None
    api_base: str | None
    api_key: str | None
    timeout: float | None
    query: list[list[str]]
    metadata: dict[str, JsonValue] | None
    principal: JsonValue
    headers: list[list[JsonValue]] | None
    details: dict[str, JsonValue]
    error: str | None


@dataclass
class Logging:
    model: str | None
    user: str | None
    provider: str | None


@dataclass
class TokenScript:
    tokens: list[str]
    projects: list[str]
    calls: list[JsonValue] = field(default_factory=list)

    def resolve(self, credentials: JsonValue, project: str | None) -> tuple[str, str]:
        self.calls.append({"credentials": credentials, "project": project})
        index = len(self.calls) - 1
        if index >= len(self.tokens):
            raise RuntimeError("the script has no token left")
        resolved = "resolved-project"
        if index < len(self.projects):
            resolved = self.projects[index]
        return self.tokens[index], resolved


class ClientSocket:
    def __init__(self, *, beta: bool) -> None:
        headers: list[tuple[bytes, bytes]] = []
        if beta:
            headers.append((b"openai-beta", b"realtime=v1"))
        self.scope: dict[str, list[tuple[bytes, bytes]]] = {"headers": headers}


def first_text(*values: str | None) -> str | None:
    for value in values:
        if value is not None and value != "":
            return value
    return None


def first_json(*values: JsonValue) -> JsonValue:
    for value in values:
        if value is None or value == "" or value == [] or value == {}:
            continue
        return value
    return None


def header_pairs(headers: dict[str, JsonValue]) -> list[list[JsonValue]]:
    return [[name, value] for name, value in sorted(headers.items())]


def query_pairs(query: dict[str, str] | None) -> list[list[str]]:
    if query is None:
        return []
    return [[name, value] for name, value in sorted(query.items())]


def merge_headers(params: Params) -> dict[str, JsonValue]:
    merged: dict[str, JsonValue] = {}
    headers = params.get("headers")
    if headers is not None:
        merged.update(headers)
    extra = params.get("extra_headers")
    if extra is not None:
        merged.update(extra)
    return merged


def build_metadata(params: Params) -> dict[str, JsonValue]:
    built: dict[str, JsonValue] = {}
    internal = params.get("litellm_metadata")
    if internal is not None:
        built.update(internal)
    selected: JsonValue = None
    kwargs_metadata = params.get("metadata")
    if kwargs_metadata is not None:
        selected = kwargs_metadata.get("guardrails")
    if not selected:
        selected = params.get("guardrails")
    if selected:
        built["guardrails"] = selected
    return built


def put_env(name: str, value: str | None) -> None:
    os.environ.pop(name, None)
    if value is not None:
        os.environ[name] = value


def apply_globals(settings: Settings) -> None:
    litellm.api_base = settings.get("api_base")
    litellm.api_key = settings.get("api_key")
    litellm.openai_key = settings.get("openai_key")
    litellm.xai_key = settings.get("xai_key")
    litellm.vertex_project = settings.get("vertex_project")
    litellm.vertex_location = settings.get("vertex_location")
    put_env("AZURE_API_BASE", settings.get("azure_api_base"))
    put_env("AZURE_API_KEY", settings.get("azure_api_key"))
    put_env("OPENAI_API_KEY", settings.get("openai_api_key"))
    put_env("XAI_API_BASE", settings.get("xai_api_base"))
    put_env("XAI_API_KEY", settings.get("xai_api_key"))
    put_env("VERTEXAI_PROJECT", settings.get("vertexai_project"))
    put_env("VERTEXAI_LOCATION", settings.get("vertexai_location"))
    put_env("VERTEXAI_CREDENTIALS", settings.get("vertexai_credentials"))
    put_env("LITELLM_AZURE_REALTIME_PROTOCOL", settings.get("azure_realtime_protocol"))


def logging_record(inputs: CallInputs) -> Logging:
    return Logging(
        model=inputs.resolved.model,
        user=inputs.params.get("user"),
        provider=inputs.resolved.provider,
    )


def to_json(branch: Branch, logging: Logging) -> JsonValue:
    payload: dict[str, JsonValue] = {
        "branch": branch.name,
        "model": branch.model,
        "api_base": branch.api_base,
        "api_key": branch.api_key,
        "timeout": branch.timeout,
        "query": branch.query,
        "metadata": branch.metadata,
        "principal": branch.principal,
        "headers": branch.headers,
        "client_is_none": True,
        "logging": {
            "model": logging.model,
            "user": logging.user,
            "custom_provider": logging.provider,
            "arealtime": True,
        },
        "details": branch.details,
        "error": branch.error,
    }
    return JSON.validate_python(payload)


def provider_branch(inputs: CallInputs) -> Branch:
    config = ProviderConfigManager.get_provider_realtime_config(
        model=inputs.resolved.model, provider=LlmProviders(inputs.resolved.provider)
    )
    name = type(config).__name__
    if name == "GeminiRealtimeConfig":
        tag = "gemini"
    elif name == "MetaRealtimeConfig":
        tag = "meta"
    else:
        raise ValueError(f"unexpected realtime config {name}")
    return Branch(
        name="provider",
        model=inputs.resolved.model,
        api_base=inputs.params.get("api_base"),
        api_key=inputs.params.get("api_key"),
        timeout=inputs.params.get("timeout"),
        query=query_pairs(inputs.query),
        metadata=build_metadata(inputs.params),
        principal=inputs.params.get("user_api_key_dict"),
        headers=header_pairs(merge_headers(inputs.params)),
        details={"tag": tag},
        error=None,
    )


def azure_branch(inputs: CallInputs) -> Branch:
    settings = inputs.settings
    params = inputs.params
    api_base = first_text(
        inputs.resolved.api_base,
        params.get("generic_api_base"),
        settings.get("api_base"),
        settings.get("azure_api_base"),
        get_secret_str("AZURE_API_BASE"),
    )
    api_key = first_text(
        inputs.resolved.api_key,
        settings.get("api_key"),
        settings.get("openai_key"),
        get_secret_str("AZURE_API_KEY"),
    )
    api_version = first_text(params.get("api_version"), params.get("generic_api_version"))
    if api_version is None:
        api_version = AZURE_DEFAULT_API_VERSION
    configured = first_text(
        params.get("realtime_protocol"),
        params.get("generic_realtime_protocol"),
        get_secret_str("LITELLM_AZURE_REALTIME_PROTOCOL"),
    )
    protocol = azure_realtime_protocol_for_client(
        configured, query_params=inputs.query, websocket=ClientSocket(beta=inputs.beta)
    )
    details: dict[str, JsonValue] = {
        "api_version": api_version,
        "protocol": protocol,
        "azure_ad_token": None,
        "ad_lookup": None,
    }
    return Branch(
        name="azure",
        model=inputs.resolved.model,
        api_base=api_base,
        api_key=api_key,
        timeout=params.get("timeout"),
        query=query_pairs(inputs.query),
        metadata=build_metadata(params),
        principal=params.get("user_api_key_dict"),
        headers=None,
        details=details,
        error=None,
    )


def openai_branch(inputs: CallInputs) -> Branch:
    settings = inputs.settings
    params = inputs.params
    api_base = first_text(
        inputs.resolved.api_base, params.get("generic_api_base"), settings.get("api_base")
    )
    if api_base is None:
        api_base = OPENAI_DEFAULT_API_BASE
    api_key = first_text(
        inputs.resolved.api_key,
        settings.get("api_key"),
        settings.get("openai_key"),
        get_secret_str("OPENAI_API_KEY"),
    )
    return Branch(
        name="openai",
        model=inputs.resolved.model,
        api_base=api_base,
        api_key=api_key,
        timeout=params.get("timeout"),
        query=query_pairs(inputs.query),
        metadata=build_metadata(params),
        principal=params.get("user_api_key_dict"),
        headers=None,
        details={},
        error=None,
    )


def bedrock_branch(inputs: CallInputs) -> Branch:
    params = inputs.params
    details: dict[str, JsonValue] = {}
    for field_name in AWS_FIELDS:
        details[field_name] = params.get(field_name)
    return Branch(
        name="bedrock",
        model=inputs.resolved.model,
        api_base=first_text(inputs.resolved.api_base, params.get("api_base")),
        api_key=first_text(inputs.resolved.api_key, params.get("api_key")),
        timeout=params.get("timeout"),
        query=[],
        metadata=None,
        principal=None,
        headers=None,
        details=details,
        error=None,
    )


def xai_branch(inputs: CallInputs) -> Branch:
    settings = inputs.settings
    params = inputs.params
    api_base = first_text(
        inputs.resolved.api_base,
        params.get("generic_api_base"),
        settings.get("xai_api_base"),
        get_secret_str("XAI_API_BASE"),
    )
    if api_base is None:
        api_base = XAI_DEFAULT_API_BASE
    api_key = XAIModelInfo.get_api_key(
        inputs.resolved.api_key, legacy_generic_before_env=True
    )
    return Branch(
        name="xai",
        model=inputs.resolved.model,
        api_base=api_base,
        api_key=api_key,
        timeout=params.get("timeout"),
        query=query_pairs(inputs.query),
        metadata=build_metadata(params),
        principal=params.get("user_api_key_dict"),
        headers=None,
        details={},
        error=None,
    )


def vertex_branch(inputs: CallInputs) -> Branch:
    settings = inputs.settings
    params = inputs.params
    credentials = first_json(
        params.get("vertex_credentials"),
        params.get("vertex_ai_credentials"),
        settings.get("vertexai_credentials"),
        get_secret_str("VERTEXAI_CREDENTIALS"),
    )
    project = first_text(
        params.get("vertex_project"),
        params.get("vertex_ai_project"),
        settings.get("vertex_project"),
        get_secret_str("VERTEXAI_PROJECT"),
    )
    location = first_text(
        params.get("vertex_location"),
        params.get("vertex_ai_location"),
        settings.get("vertex_location"),
        get_secret_str("VERTEXAI_LOCATION"),
    )
    access_token, resolved_project = inputs.script.resolve(credentials, project)
    refreshed, _ = inputs.script.resolve(credentials, resolved_project)
    details: dict[str, JsonValue] = {
        "access_token": access_token,
        "project": resolved_project,
        "location": location,
        "refresh_token": refreshed,
        "token_calls": inputs.script.calls,
    }
    return Branch(
        name="vertex",
        model=inputs.resolved.model,
        api_base=inputs.resolved.api_base,
        api_key=None,
        timeout=params.get("timeout"),
        query=query_pairs(inputs.query),
        metadata=build_metadata(params),
        principal=params.get("user_api_key_dict"),
        headers=header_pairs(merge_headers(params)),
        details=details,
        error=None,
    )


def script_for(scenario: Scenario) -> TokenScript:
    setup = scenario.get("token")
    if setup is None:
        return TokenScript(tokens=[], projects=[])
    tokens = setup.get("tokens")
    if tokens is None:
        tokens = []
    projects = setup.get("projects")
    if projects is None:
        projects = []
    return TokenScript(tokens=tokens, projects=projects)


def known_branch(inputs: CallInputs) -> Branch | None:
    provider = inputs.resolved.provider
    if provider in ("gemini", "meta"):
        return provider_branch(inputs)
    if provider in ("azure", "azure_ai"):
        return azure_branch(inputs)
    if provider == "openai":
        return openai_branch(inputs)
    if provider == "bedrock":
        return bedrock_branch(inputs)
    if provider == "xai":
        return xai_branch(inputs)
    if provider == "vertex_ai":
        return vertex_branch(inputs)
    return None


def error_branch(model: str) -> Branch:
    return Branch(
        name="error",
        model=None,
        api_base=None,
        api_key=None,
        timeout=None,
        query=[],
        metadata=None,
        principal=None,
        headers=None,
        details={},
        error=f"Unsupported model: {model}",
    )


def run_scenario(scenario: Scenario) -> JsonValue:
    settings = scenario.get("settings")
    if settings is None:
        settings = Settings()
    params = scenario.get("params")
    if params is None:
        params = Params()
    beta = scenario.get("client_sent_beta")
    if beta is None:
        beta = False
    apply_globals(settings)
    model, provider, dynamic_api_key, dynamic_api_base = get_llm_provider(
        model=scenario["model"],
        api_base=params.get("api_base"),
        api_key=params.get("api_key"),
    )
    resolved = Resolved(
        model=model,
        provider=provider,
        api_base=dynamic_api_base,
        api_key=dynamic_api_key,
    )
    query = scenario.get("query_params")
    if query is not None:
        copied = dict(query)
        if "model" in copied:
            copied["model"] = model
        query = copied
    inputs = CallInputs(
        params=params,
        settings=settings,
        resolved=resolved,
        query=query,
        beta=beta,
        script=script_for(scenario),
    )
    branch = known_branch(inputs)
    if branch is None:
        branch = error_branch(model)
    return to_json(branch, logging_record(inputs))


def run(argument: Input) -> JsonValue:
    return {"scenarios": [run_scenario(scenario) for scenario in argument["scenarios"]]}
