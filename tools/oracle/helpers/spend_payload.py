import json
import os
from datetime import datetime
from typing import Literal, NotRequired, TypedDict

import litellm
from litellm.proxy import proxy_server
from litellm.proxy.route_llm_request import ProxyModelNotFoundError
from litellm.proxy.spend_tracking.spend_tracking_utils import get_logging_payload  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_tracking_utils.py#L498
from litellm.router import Router
from litellm.utils import get_end_user_id_for_cost_tracking  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L10070
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True
OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
TAGS: TypeAdapter[list[JsonValue]] = TypeAdapter(list[JsonValue])


class SettingsInput(TypedDict):
    disable_end_user_cost_tracking: NotRequired[bool | None]
    # ast-grep-ignore: no-long-names
    enable_end_user_cost_tracking_prometheus_only: NotRequired[bool | None]
    store_bodies: NotRequired[bool]
    turn_off_message_logging: NotRequired[bool]
    max_string_length: NotRequired[int]
    missing_session_id: NotRequired[Literal["generate", "reject", "omit"] | None]


class DeploymentInput(TypedDict):
    model_name: str
    litellm_params: NotRequired[dict[str, JsonValue]]


class RouterInput(TypedDict):
    model_list: NotRequired[list[DeploymentInput]]
    recognized: NotRequired[list[str]]
    team_public: NotRequired[list[str]]


class ResponseMapping(TypedDict):
    mapping: dict[str, JsonValue]


ResponseInput = Literal["absent", "other", "unknown_model_error"] | ResponseMapping


class PayloadInput(TypedDict):
    function: Literal["payload"]
    kwargs: dict[str, JsonValue]
    response: NotRequired[ResponseInput]
    start_time: str
    end_time: str
    settings: NotRequired[SettingsInput]
    router: NotRequired[RouterInput]
    clock: NotRequired[float]
    session_uuid: NotRequired[str]


class EndUserInput(TypedDict):
    function: Literal["end_user"]
    litellm_params: dict[str, JsonValue]
    service: Literal["litellm_logging", "prometheus"]
    settings: NotRequired[SettingsInput]


class PayloadView(TypedDict):
    request_id: str
    call_type: str
    api_key: str
    spend: JsonValue
    total_tokens: int | None
    prompt_tokens: int | None
    completion_tokens: int | None
    startTime: datetime
    endTime: datetime
    completionStartTime: datetime | None
    model: str
    model_id: str | None
    model_group: str | None
    mcp_namespaced_tool_name: str | None
    billing_agent_id: str | None
    agent_id: str | None
    api_base: str
    user: str
    metadata: str
    cache_hit: str
    cache_key: str
    request_tags: str
    team_id: str | None
    organization_id: str | None
    end_user: str | None
    requester_ip_address: str | None
    custom_llm_provider: str | None
    messages: JsonValue
    response: JsonValue
    proxy_server_request: str | None
    session_id: str | None
    request_duration_ms: int | None
    status: Literal["success", "failure"]
    litellm_call_id: str | None


PAYLOAD: TypeAdapter[PayloadView] = TypeAdapter(PayloadView)


def run(input_value: PayloadInput | EndUserInput) -> JsonValue:
    if input_value["function"] == "end_user":
        return end_user(input_value)
    return payload(input_value)


def end_user(input_value: EndUserInput) -> str | None:
    settings = input_value.get("settings")
    if settings is None:
        settings = SettingsInput()
    apply_settings(settings)
    params: dict[str, JsonValue] = dict(input_value["litellm_params"])
    if input_value["service"] == "prometheus":
        return get_end_user_id_for_cost_tracking(params, service_type="prometheus")
    return get_end_user_id_for_cost_tracking(params, service_type="litellm_logging")


def payload(input_value: PayloadInput) -> JsonValue:
    settings = input_value.get("settings")
    if settings is None:
        settings = SettingsInput()
    apply_settings(settings)
    litellm.cache = None
    router = build_router(input_value.get("router"))
    kwargs: dict[str, JsonValue | datetime] = {}
    kwargs.update(input_value["kwargs"])
    if "completion_start_time" in input_value["kwargs"]:
        completion_start_time = input_value["kwargs"]["completion_start_time"]
        if completion_start_time is None:
            kwargs["completion_start_time"] = None
        else:
            match completion_start_time:
                case str() as text:
                    kwargs["completion_start_time"] = datetime.fromisoformat(text)
                case bool() | int() | float() | list() | dict():
                    raise ValueError("completion_start_time must be an ISO 8601 string")
    record = PAYLOAD.validate_python(
        get_logging_payload(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_tracking_utils.py#L498-L504
            kwargs=kwargs,
            response_obj=response_of(input_value.get("response")),
            start_time=datetime.fromisoformat(input_value["start_time"]),
            end_time=datetime.fromisoformat(input_value["end_time"]),
            llm_router=router,
        )
    )
    return normalize(record)


def apply_settings(settings: SettingsInput) -> None:
    litellm.disable_end_user_cost_tracking = settings.get("disable_end_user_cost_tracking")
    litellm.enable_end_user_cost_tracking_prometheus_only = settings.get(
        "enable_end_user_cost_tracking_prometheus_only"
    )
    turn_off_message_logging = settings.get("turn_off_message_logging")
    if turn_off_message_logging is None:
        turn_off_message_logging = False
    litellm.turn_off_message_logging = turn_off_message_logging
    store_bodies = settings.get("store_bodies")
    if store_bodies is None:
        store_bodies = False
    proxy_server.general_settings["store_prompts_in_spend_logs"] = store_bodies  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L2596
    proxy_server.general_settings["missing_session_id"] = settings.get("missing_session_id")  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L2596
    os.environ["STORE_PROMPTS_IN_SPEND_LOGS"] = "true" if store_bodies else "false"
    max_string_length = settings.get("max_string_length")
    if max_string_length is None:
        os.environ.pop("MAX_STRING_LENGTH_PROMPT_IN_DB", None)
    else:
        os.environ["MAX_STRING_LENGTH_PROMPT_IN_DB"] = str(max_string_length)


def build_router(input_router: RouterInput | None) -> Router | None:
    if input_router is None:
        return None
    entries: list[dict[str, JsonValue]] = []
    deployments = input_router.get("model_list")
    if deployments is not None:
        for deployment in deployments:
            entry: dict[str, JsonValue] = {"model_name": deployment["model_name"]}
            params = deployment.get("litellm_params")
            if params is not None:
                entry["litellm_params"] = dict(params)
            entries.append(entry)
    router = Router(model_list=entries)
    team_public = input_router.get("team_public")
    if team_public is not None:
        router.team_public_model_names = frozenset(team_public)
    return router


def response_of(
    response: ResponseInput | None,
) -> ProxyModelNotFoundError | str | dict[str, JsonValue] | None:
    if response is None:
        return None
    if response == "absent":
        return None
    if response == "other":
        return "not-a-mapping"
    if response == "unknown_model_error":
        return ProxyModelNotFoundError(route="/chat/completions", model_name="missing")
    return response["mapping"]


def normalize(record: PayloadView) -> JsonValue:
    completion_start_time = record["completionStartTime"]
    normalized: dict[str, JsonValue] = {
        "request_id": record["request_id"],
        "call_type": record["call_type"],
        "api_key": record["api_key"],
        "spend": record["spend"],
        "total_tokens": record["total_tokens"],
        "prompt_tokens": record["prompt_tokens"],
        "completion_tokens": record["completion_tokens"],
        "startTime": record["startTime"].isoformat(),
        "endTime": record["endTime"].isoformat(),
        "completionStartTime": (
            completion_start_time.isoformat() if completion_start_time is not None else None
        ),
        "model": record["model"],
        "model_id": record["model_id"],
        "model_group": record["model_group"],
        "mcp_namespaced_tool_name": record["mcp_namespaced_tool_name"],
        "billing_agent_id": record["billing_agent_id"],
        "agent_id": record["agent_id"],
        "api_base": record["api_base"],
        "user": record["user"],
        "metadata": json.dumps(
            OBJECT.validate_json(record["metadata"]), separators=(",", ":"), sort_keys=True
        ),
        "cache_hit": record["cache_hit"],
        "cache_key": record["cache_key"],
        "request_tags": json.dumps(
            TAGS.validate_json(record["request_tags"]), separators=(",", ":"), sort_keys=True
        ),
        "team_id": record["team_id"],
        "organization_id": record["organization_id"],
        "end_user": record["end_user"],
        "requester_ip_address": record["requester_ip_address"],
        "custom_llm_provider": record["custom_llm_provider"],
        "messages": record["messages"],
        "response": record["response"],
        "proxy_server_request": record["proxy_server_request"],
        "session_id": record["session_id"],
        "request_duration_ms": record["request_duration_ms"],
        "status": record["status"],
        "litellm_call_id": record["litellm_call_id"],
    }
    return normalized
