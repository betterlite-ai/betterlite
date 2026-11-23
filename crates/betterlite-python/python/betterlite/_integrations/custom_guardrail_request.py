from collections.abc import Mapping

from pydantic import JsonValue

from betterlite._integrations.custom_guardrail_constants import RECORDS_KEY
from betterlite._integrations.custom_guardrail_support import (
    as_list,
    as_mapping,
    json_projection,
    synced_target,
)
from betterlite._integrations.custom_logger_types import LiteLLMLoggingObj
from betterlite._integrations.guardrail_types import LogTarget
from betterlite.exceptions._process import (
    BlockedPiiEntityError,
    GuardrailRaisedException,
    ModifyResponseException,
    SensitiveDataRouteException,
)


def project_mapping(value: JsonValue) -> dict[str, JsonValue] | None:
    mapping = as_mapping(value)
    if mapping is None:
        return None
    return {name: json_projection(item) for name, item in mapping.items()}


def log_target_json(logging_obj: LiteLLMLoggingObj | None) -> LogTarget | None:
    if logging_obj is None:
        return None
    # ast-grep-ignore: no-reflection
    params: JsonValue = getattr(logging_obj, "litellm_params", None)
    # ast-grep-ignore: no-reflection
    details: JsonValue = getattr(logging_obj, "model_call_details", None)
    detail_map = project_mapping(details)
    call_params = None if detail_map is None else project_mapping(detail_map.get("litellm_params"))
    return LogTarget(litellm_params=project_mapping(params), model_call_params=call_params)


def apply_log_target(logging_obj: LiteLLMLoggingObj | None, target: LogTarget | None) -> None:
    if logging_obj is None or target is None:
        return
    params = target["litellm_params"]
    if params is None:
        return
    metadata = as_mapping(params.get("metadata"))
    # ast-grep-ignore: no-reflection
    live: JsonValue = getattr(logging_obj, "litellm_params", None)
    # ast-grep-ignore: no-reflection
    details: JsonValue = getattr(logging_obj, "model_call_details", None)
    detail_map = as_mapping(details)
    nested = None if detail_map is None else as_mapping(detail_map.get("litellm_params"))
    for view in (as_mapping(live), nested):
        if view is not None and metadata is not None:
            view["metadata"] = metadata


def apply_records(request_data: dict[str, JsonValue], updated: JsonValue) -> None:
    bucket_name = "litellm_metadata" if "litellm_metadata" in request_data else "metadata"
    outer = as_mapping(updated)
    if outer is None:
        return
    bucket = as_mapping(outer.get(bucket_name))
    if bucket is None:
        return
    entries = bucket.get(RECORDS_KEY)
    live = as_mapping(request_data.get(bucket_name))
    if live is None:
        request_data[bucket_name] = bucket
        return
    live[RECORDS_KEY] = entries


def sync_guardrail_info(
    request_data: dict[str, JsonValue], logging_obj: LiteLLMLoggingObj | None
) -> None:
    target = log_target_json(logging_obj)
    if target is None:
        return
    apply_log_target(logging_obj, synced_target(request_data, target))


def session_text(value: JsonValue) -> str | None:
    match value:
        case str():
            return value
        case bool() | int() | float():
            return str(value)
        case list() | dict() | None:
            return None


def session_id_from_request_data(request_data: Mapping[str, JsonValue]) -> str | None:
    direct = session_text(request_data.get("litellm_session_id"))
    if direct is not None:
        return direct
    for bucket in ("metadata", "litellm_metadata"):
        metadata = as_mapping(request_data.get(bucket))
        if metadata is None:
            continue
        nested = session_text(metadata.get("session_id"))
        if nested is not None:
            return nested
    return None


def admin_metadata(data: Mapping[str, JsonValue]) -> dict[str, JsonValue]:
    team: dict[str, JsonValue] = {}
    key: dict[str, JsonValue] = {}
    for bucket_name in ("metadata", "litellm_metadata"):
        bucket = as_mapping(data.get(bucket_name))
        if bucket is None:
            continue
        team = as_mapping(bucket.get("user_api_key_team_metadata")) or team
        key = as_mapping(bucket.get("user_api_key_metadata")) or key
    return {**team, **key}


def requested_list(value: JsonValue) -> list[JsonValue]:
    listed = as_list(value)
    if listed is None:
        raise TypeError("guardrails must be a list")
    return listed


def is_guardrail_intervention(e: Exception) -> bool:
    match e:
        case (
            ModifyResponseException()
            | GuardrailRaisedException()
            | BlockedPiiEntityError()
            | SensitiveDataRouteException()
        ):
            return True
        case Exception():
            return False
