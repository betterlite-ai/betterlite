import json
import os
from datetime import datetime
from typing import Literal, NotRequired, TypedDict

import litellm
from litellm.proxy import proxy_server
from litellm.proxy._types import SpendLogsPayload
from litellm.proxy.route_llm_request import ProxyModelNotFoundError
from litellm.proxy.spend_tracking.spend_tracking_utils import get_logging_payload
from litellm.router import Router
from litellm.utils import get_end_user_id_for_cost_tracking
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True
OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class DeploymentInput(TypedDict):
    model_name: str
    litellm_params: NotRequired[dict[str, JsonValue]]


class RouterInput(TypedDict):
    model_list: NotRequired[list[DeploymentInput]]
    recognized: NotRequired[list[str]]
    team_public: NotRequired[list[str]]


class SettingsInput(TypedDict):
    disable_end_user_cost_tracking: NotRequired[bool | None]
    enable_end_user_cost_tracking_prometheus_only: NotRequired[bool | None]
    store_bodies: NotRequired[bool]
    turn_off_message_logging: NotRequired[bool]


class PayloadInput(TypedDict):
    function: Literal["payload"]
    kwargs: dict[str, JsonValue]
    response: NotRequired[JsonValue]
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


def run(input_value: PayloadInput | EndUserInput) -> JsonValue:
    if input_value["function"] == "end_user":
        return end_user(input_value)
    return payload(input_value)


def payload(input_value: PayloadInput) -> JsonValue:
    apply_settings(input_value.get("settings", {}))
    litellm.cache = None
    router = build_router(input_value.get("router"))
    kwargs: dict[str, object] = dict(input_value["kwargs"])
    completion_start_time = kwargs.get("completion_start_time")
    if isinstance(completion_start_time, str):
        kwargs["completion_start_time"] = datetime.fromisoformat(completion_start_time)
    record = get_logging_payload(
        kwargs=kwargs,
        response_obj=response_of(input_value.get("response", "absent")),
        start_time=datetime.fromisoformat(input_value["start_time"]),
        end_time=datetime.fromisoformat(input_value["end_time"]),
        llm_router=router,
    )
    return normalize(record)


def end_user(input_value: EndUserInput) -> str | None:
    apply_settings(input_value.get("settings", {}))
    params: dict = dict(input_value["litellm_params"])
    if input_value["service"] == "prometheus":
        return get_end_user_id_for_cost_tracking(params, service_type="prometheus")
    return get_end_user_id_for_cost_tracking(params, service_type="litellm_logging")


def apply_settings(settings: SettingsInput) -> None:
    litellm.disable_end_user_cost_tracking = settings.get("disable_end_user_cost_tracking")
    litellm.enable_end_user_cost_tracking_prometheus_only = settings.get(
        "enable_end_user_cost_tracking_prometheus_only"
    )
    litellm.turn_off_message_logging = settings.get("turn_off_message_logging", False)
    store_bodies = settings.get("store_bodies", False)
    proxy_server.general_settings["store_prompts_in_spend_logs"] = store_bodies
    os.environ["STORE_PROMPTS_IN_SPEND_LOGS"] = "true" if store_bodies else "false"


def build_router(input_router: RouterInput | None) -> Router | None:
    if input_router is None:
        return None
    entries: list[dict[str, JsonValue]] = []
    for deployment in input_router.get("model_list", []):
        entry: dict[str, JsonValue] = {}
        entry["model_name"] = deployment["model_name"]
        entry["litellm_params"] = dict(deployment.get("litellm_params", {}))
        entries.append(entry)
    router = Router(model_list=entries)
    router.team_public_model_names = frozenset(input_router.get("team_public", []))
    return router


def response_of(value: JsonValue) -> object:
    if value == "unknown_model_error":
        return ProxyModelNotFoundError(route="/chat/completions", model_name="missing")
    if value == "other":
        return "not-a-mapping"
    if isinstance(value, dict):
        return OBJECT.validate_python(value.get("mapping"))
    return None


def normalize(record: SpendLogsPayload) -> dict[str, JsonValue]:
    record["startTime"] = isoformat(record["startTime"])
    record["endTime"] = isoformat(record["endTime"])
    completion_start_time = record["completionStartTime"]
    if completion_start_time is not None:
        record["completionStartTime"] = isoformat(completion_start_time)
    record["metadata"] = compact(record["metadata"])
    record["request_tags"] = compact(record["request_tags"])
    return OBJECT.validate_python(record)


def isoformat(value: datetime | str) -> str:
    if isinstance(value, datetime):
        return value.isoformat()
    return value


def compact(value: str) -> str:
    return json.dumps(json.loads(value), separators=(",", ":"))
