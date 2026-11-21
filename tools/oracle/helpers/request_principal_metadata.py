from typing import Final, Literal, TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.litellm_pre_call_utils import LiteLLMProxyRequestSetup
from litellm.proxy.proxy_server import ProxyConfig
from litellm.types.proxy.agent_identity import ManagedAgentContext
from pydantic import JsonValue, TypeAdapter, ValidationError
from starlette.requests import Request
from starlette.types import Scope

Slot = Literal["metadata", "litellm_metadata"]


# the principal travels whole into UserAPIKeyAuth, and only its metadata is read here
class Principal(TypedDict):
    metadata: dict[str, JsonValue]


class PreAuthCall(TypedDict):
    data: dict[str, JsonValue]
    headers: list[tuple[str, str]]
    route: str
    principal: Principal


class AgentContext(TypedDict):
    agent_id: str
    mode: Literal["autonomous", "delegated"]
    user_id: str | None


class MetadataCall(TypedDict):
    data: dict[str, JsonValue]
    slot: Slot
    bucket: dict[str, JsonValue]
    headers: list[tuple[str, str]]
    principal: Principal
    invoked_agent_id: str | None
    managed_agent_context: AgentContext | None


class TeamCall(TypedDict):
    team_id: str
    teams: list[dict[str, JsonValue]]


class Input(TypedDict):
    pre_auth_tags: list[PreAuthCall]
    request_metadata: list[MetadataCall]
    team_callbacks: list[TeamCall]


class AuthView(TypedDict):
    metadata: dict[str, JsonValue]
    team_metadata: dict[str, JsonValue] | None
    project_metadata: dict[str, JsonValue] | None
    organization_metadata: dict[str, JsonValue] | None


class TeamCallbacks(TypedDict):
    success_callback: list[str] | None
    failure_callback: list[str] | None
    callback_vars: dict[str, str] | None


JSON: Final = TypeAdapter(JsonValue)
AUTH: Final[TypeAdapter[UserAPIKeyAuth]] = TypeAdapter(UserAPIKeyAuth)
AUTH_VIEW: Final = TypeAdapter(AuthView)
TEAM_CALLBACKS: Final = TypeAdapter(TeamCallbacks)


def request_for(headers: list[tuple[str, str]]) -> Request:
    raw_headers = [(name.lower().encode(), value.encode()) for name, value in headers]
    scope: Scope = {
        "type": "http",
        "method": "POST",
        "path": "/",
        "query_string": b"",
        "headers": raw_headers,
    }
    return Request(scope=scope)


def pre_auth_tags(call: PreAuthCall) -> JsonValue:
    data = call["data"]
    principal = AUTH.validate_python(call["principal"])
    LiteLLMProxyRequestSetup.pre_seed_litellm_metadata_for_route(  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1855
        data, call["route"]
    )
    LiteLLMProxyRequestSetup.apply_client_tag_policy_pre_auth(  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1899
        request_for(call["headers"]), data, principal
    )
    LiteLLMProxyRequestSetup.apply_key_tags_pre_auth(  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1876
        data, principal
    )
    outcome = {"data": data}
    return JSON.validate_python(outcome)


def request_metadata(call: MetadataCall) -> JsonValue:
    principal = AUTH.validate_python(call["principal"])
    principal.invoked_agent_id = call["invoked_agent_id"]
    context = call["managed_agent_context"]
    if context is not None:
        principal.managed_agent_context = ManagedAgentContext.model_validate(context)
    bucket = call["bucket"]
    data: dict[str, JsonValue] = {**call["data"], call["slot"]: bucket}
    LiteLLMProxyRequestSetup.add_user_api_key_auth_to_request_metadata(  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1653
        data, principal, call["slot"]
    )
    LiteLLMProxyRequestSetup.add_key_level_controls(  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1735
        call["principal"]["metadata"], data, call["slot"]
    )
    request_tags = LiteLLMProxyRequestSetup.add_request_tag_to_metadata(  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1834
        None, dict(call["headers"]), data
    )
    auth_view = AUTH.validate_python(bucket.pop("user_api_key_auth"))
    outcome = {
        "data": data,
        "auth_view": AUTH_VIEW.validate_json(auth_view.model_dump_json()),
        "request_tags": request_tags,
    }
    return JSON.validate_python(outcome)


def team_callbacks(call: TeamCall) -> JsonValue:
    proxy_config = ProxyConfig()
    settings = {"litellm_settings": {"default_team_settings": call["teams"]}}
    proxy_config.update_config_state(settings)
    try:
        callbacks = LiteLLMProxyRequestSetup.add_team_based_callbacks_from_config(
            call["team_id"], proxy_config
        )
    except ValidationError:
        error = {"error": "invalid callback variable"}
        return JSON.validate_python(error)
    if callbacks is None:
        return None
    return JSON.validate_python(TEAM_CALLBACKS.validate_json(callbacks.model_dump_json()))


def run(input_value: Input) -> JsonValue:
    outcome = {
        "pre_auth_tags": [pre_auth_tags(call) for call in input_value["pre_auth_tags"]],
        "request_metadata": [
            request_metadata(call) for call in input_value["request_metadata"]
        ],
        "team_callbacks": [team_callbacks(call) for call in input_value["team_callbacks"]],
    }
    return JSON.validate_python(outcome)
