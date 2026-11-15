import asyncio
import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from typing import Final, NotRequired, TypedDict

import litellm
from litellm.llms.azure import common_utils as azure_common
from litellm.llms.azure_ai.common_utils import (
    AzureAIApiKeyHeader,
    AzureFoundryModelInfo,
    api_key_header_for_base,
    azure_ai_supports_native_responses,
    foundry_chat_rejects_function_tools_while_reasoning,
    get_azure_ai_agent_entra_token,
    get_azure_ai_auth_headers,
    get_azure_ai_entra_token,
    has_azure_entra_params,
    is_azure_openai_v1_host,
    is_foundry_model_inference_base,
    resolve_azure_ai_agent_auth_header,
)
from pydantic import JsonValue

DEFAULT_SCOPE: Final[str] = "https://cognitiveservices.azure.com/.default"


class HostStep(TypedDict):
    api_base: str


class AuthStep(TypedDict, total=False):
    api_key: str
    api_key_env_var: str
    environment: dict[str, str]
    header: AzureAIApiKeyHeader
    params: dict[str, JsonValue]


class AgentStep(TypedDict, total=False):
    environment: dict[str, str]
    params: dict[str, JsonValue]


class SettingsStep(TypedDict, total=False):
    api_base: str
    api_key: str
    environment: dict[str, str]
    global_api_base: str
    global_api_key: str
    params: dict[str, JsonValue]


class NativeStep(TypedDict, total=False):
    api_base: str
    environment: dict[str, str]
    global_api_base: str
    model: str


class GateStep(TypedDict):
    model: str
    reasoning_effort: NotRequired[str | dict[str, JsonValue]]


class Input(TypedDict, total=False):
    agent_headers: list[AgentStep]
    agent_tokens: list[AgentStep]
    auth_headers: list[AuthStep]
    has_entra: list[AgentStep]
    host_boundary: list[HostStep]
    native_responses: list[NativeStep]
    reasoning_gate: list[GateStep]
    settings: list[SettingsStep]


class ScriptedToken:
    def __init__(self, token: str) -> None:
        self.token = token

    def __call__(self) -> str:
        return self.token


def client_secret_grant(
    tenant_id: str,
    client_id: str,
    client_secret: str,
    scope: str = DEFAULT_SCOPE,
) -> Callable[[], str]:
    return ScriptedToken(
        f"client_secret tenant={tenant_id} client={client_id} secret={client_secret} scope={scope}"
    )


def username_password_grant(
    client_id: str,
    azure_username: str,
    azure_password: str,
    scope: str = DEFAULT_SCOPE,
) -> Callable[[], str]:
    fields = [
        "username_password",
        f"client={client_id}",
        f"user={azure_username}",
        f"password={azure_password}",
        f"scope={scope}",
    ]
    return ScriptedToken(" ".join(fields))


def oidc_grant(
    azure_ad_token: str,
    azure_client_id: str | None = None,
    azure_tenant_id: str | None = None,
    scope: str | None = None,
) -> str:
    claim = f"assertion={azure_ad_token}"
    return f"oidc {claim} client={azure_client_id} tenant={azure_tenant_id} scope={scope}"


@contextmanager
def grant_overrides() -> Generator[None]:
    saved = (
        azure_common.get_azure_ad_token_from_entra_id,
        azure_common.get_azure_ad_token_from_username_password,
        azure_common.get_azure_ad_token_from_oidc,
    )
    azure_common.get_azure_ad_token_from_entra_id = client_secret_grant
    azure_common.get_azure_ad_token_from_username_password = username_password_grant
    azure_common.get_azure_ad_token_from_oidc = oidc_grant
    try:
        yield
    finally:
        (
            azure_common.get_azure_ad_token_from_entra_id,
            azure_common.get_azure_ad_token_from_username_password,
            azure_common.get_azure_ad_token_from_oidc,
        ) = saved


@contextmanager
def environment(values: dict[str, str]) -> Generator[None]:
    saved: dict[str, str | None] = {name: os.environ.get(name) for name in values}
    os.environ.update(values)
    try:
        yield
    finally:
        for name, previous in saved.items():
            if previous is None:
                del os.environ[name]
            else:
                os.environ[name] = previous


@contextmanager
def global_settings(api_base: str | None, api_key: str | None) -> Generator[None]:
    saved_base = litellm.api_base
    saved_key = litellm.api_key
    saved_refresh = litellm.enable_azure_ad_token_refresh
    litellm.api_base = api_base
    litellm.api_key = api_key
    litellm.enable_azure_ad_token_refresh = False
    try:
        yield
    finally:
        litellm.api_base = saved_base
        litellm.api_key = saved_key
        litellm.enable_azure_ad_token_refresh = saved_refresh


def agent_token(params: dict[str, JsonValue]) -> JsonValue:
    token = get_azure_ai_agent_entra_token(params)
    return {"kind": "token", "token": token}


def agent_headers(params: dict[str, JsonValue]) -> JsonValue:
    headers = asyncio.run(resolve_azure_ai_agent_auth_header(params))
    return {"kind": "headers", "headers": dict(headers)}


def agent_step(
    step: AgentStep, fetch: Callable[[dict[str, JsonValue]], JsonValue]
) -> JsonValue:
    values = step.get("environment")
    if values is None:
        values = {}
    params = step.get("params")
    if params is None:
        params = {}
    with environment(values):
        try:
            reply = fetch(params)
        except (TypeError, ValueError):
            return {"kind": "error"}
    return reply


def host_step(step: HostStep) -> JsonValue:
    base = step["api_base"]
    header = api_key_header_for_base(base)
    return {
        "foundry_base": is_foundry_model_inference_base(base),
        "header": header,
        "v1_host": is_azure_openai_v1_host(base),
    }


def auth_step(step: AuthStep) -> JsonValue:
    values = step.get("environment")
    if values is None:
        values = {}
    header = step.get("header")
    if header is None:
        header = "Authorization"
    env_var = step.get("api_key_env_var")
    if env_var is None:
        env_var = "AZURE_AI_API_KEY"
    with environment(values):
        try:
            headers = get_azure_ai_auth_headers(
                step.get("api_key"), step.get("params"), header, env_var
            )
        except (TypeError, ValueError):
            return {"kind": "error"}
    return {"kind": "headers", "headers": dict(headers)}


def entra_outcome(params: dict[str, JsonValue] | None) -> JsonValue:
    try:
        token = get_azure_ai_entra_token(params)
    except (TypeError, ValueError):
        return {"kind": "error"}
    if token is None:
        return {"kind": "absent"}
    return {"kind": "token", "token": token}


def settings_step(step: SettingsStep) -> JsonValue:
    values = step.get("environment")
    if values is None:
        values = {}
    base = step.get("global_api_base")
    key = step.get("global_api_key")
    with environment(values), global_settings(base, key):
        api_base = AzureFoundryModelInfo.get_api_base(step.get("api_base"))
        api_key = AzureFoundryModelInfo.get_api_key(step.get("api_key"))
        outcome = entra_outcome(step.get("params"))
        return {"api_base": api_base, "api_key": api_key, "outcome": outcome}


def native_step(step: NativeStep) -> JsonValue:
    values = step.get("environment")
    if values is None:
        values = {}
    with environment(values), global_settings(step.get("global_api_base"), None):
        return azure_ai_supports_native_responses(step.get("model"), step.get("api_base"))


def gate_step(step: GateStep) -> JsonValue:
    effort = step.get("reasoning_effort")
    return foundry_chat_rejects_function_tools_while_reasoning(step["model"], effort)


def execute(values: Input) -> JsonValue:
    if "agent_headers" in values:
        return {
            "steps": [agent_step(step, agent_headers) for step in values["agent_headers"]],
        }
    if "agent_tokens" in values:
        return {
            "steps": [agent_step(step, agent_token) for step in values["agent_tokens"]],
        }
    if "auth_headers" in values:
        return {"steps": [auth_step(step) for step in values["auth_headers"]]}
    if "has_entra" in values:
        return {
            "steps": [has_azure_entra_params(step.get("params")) for step in values["has_entra"]],
        }
    return execute_default(values)


def execute_default(values: Input) -> JsonValue:
    steps: list[JsonValue] | None = None
    if "host_boundary" in values:
        steps = [host_step(step) for step in values["host_boundary"]]
    if "native_responses" in values:
        steps = [native_step(step) for step in values["native_responses"]]
    if "reasoning_gate" in values:
        steps = [gate_step(step) for step in values["reasoning_gate"]]
    if "settings" in values:
        steps = [settings_step(step) for step in values["settings"]]
    if steps is None:
        raise ValueError("foundry_credentials needs one operation")
    return {"steps": steps}


def run(values: Input) -> JsonValue:
    with grant_overrides():
        return execute(values)
