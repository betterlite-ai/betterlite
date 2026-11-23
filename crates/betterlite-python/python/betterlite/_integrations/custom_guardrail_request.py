import copy
from collections.abc import Mapping

from pydantic import JsonValue

from betterlite._integrations.custom_guardrail_constants import (
    PRE_CALL_CONTENT_KEYS,
    RECORDS_KEY,
)
from betterlite._integrations.custom_guardrail_support import json_projection, synced_target
from betterlite._integrations.guardrail_types import LogTarget
from betterlite.exceptions._process import (
    BlockedPiiEntityError,
    GuardrailRaisedException,
    ModifyResponseException,
    SensitiveDataRouteException,
)
from betterlite.types.guardrails.params import GuardrailEventHooks


def request_data_of(kwargs: Mapping[str, JsonValue]) -> dict[str, JsonValue]:
    data = kwargs.get("data")
    match data:
        case dict():
            return data
        case str() | bool() | int() | float() | list() | None:
            pass
    request_data = kwargs.get("request_data")
    match request_data:
        case dict():
            return request_data
        case str() | bool() | int() | float() | list() | None:
            return {}


def metadata_key(request_data: Mapping[str, JsonValue]) -> str:
    name = "litellm_metadata" if "litellm_metadata" in request_data else "metadata"
    return name


def project_mapping(value: JsonValue) -> dict[str, JsonValue] | None:
    match value:
        case dict():
            return {name: json_projection(item) for name, item in value.items()}
        case str() | bool() | int() | float() | list() | None:
            return None


def log_target_json(logging_obj: JsonValue) -> LogTarget | None:
    if logging_obj is None:
        return None
    # ast-grep-ignore: no-reflection
    params: JsonValue = getattr(logging_obj, "litellm_params", None)
    # ast-grep-ignore: no-reflection
    details: JsonValue = getattr(logging_obj, "model_call_details", None)
    detail_map = project_mapping(details)
    call_params = None if detail_map is None else project_mapping(detail_map.get("litellm_params"))
    return LogTarget(litellm_params=project_mapping(params), model_call_params=call_params)


def apply_metadata_view(container: JsonValue, params: Mapping[str, JsonValue]) -> None:
    match container:
        case dict():
            metadata = params.get("metadata")
            match metadata:
                case dict():
                    container["metadata"] = metadata
                case str() | bool() | int() | float() | list() | None:
                    return
        case str() | bool() | int() | float() | list() | None:
            return


def apply_details_view(details: JsonValue, params: Mapping[str, JsonValue]) -> None:
    match details:
        case dict():
            inner = details.get("litellm_params")
            apply_metadata_view(inner, params)
        case str() | bool() | int() | float() | list() | None:
            return


def apply_log_target(logging_obj: JsonValue, target: LogTarget | None) -> None:
    if logging_obj is None or target is None:
        return
    params = target["litellm_params"]
    if params is None:
        return
    # ast-grep-ignore: no-reflection
    live: JsonValue = getattr(logging_obj, "litellm_params", None)
    apply_metadata_view(live, params)
    # ast-grep-ignore: no-reflection
    details: JsonValue = getattr(logging_obj, "model_call_details", None)
    apply_details_view(details, params)


def apply_records(request_data: dict[str, JsonValue], updated: JsonValue) -> None:
    bucket_name = metadata_key(request_data)
    match updated:
        case dict():
            bucket = updated.get(bucket_name)
            match bucket:
                case dict():
                    entries = bucket.get(RECORDS_KEY)
                    live = request_data.get(bucket_name)
                    match live:
                        case dict():
                            live[RECORDS_KEY] = entries
                        case str() | bool() | int() | float() | list() | None:
                            request_data[bucket_name] = bucket
                case str() | bool() | int() | float() | list() | None:
                    return
        case str() | bool() | int() | float() | list() | None:
            return


def sync_guardrail_info(request_data: dict[str, JsonValue], logging_obj: JsonValue) -> None:
    target = log_target_json(logging_obj)
    if target is None:
        return
    apply_log_target(logging_obj, synced_target(request_data, target))


def original_inputs_for(
    hook_name: str,
    kwargs: Mapping[str, JsonValue],
    request_data: Mapping[str, JsonValue],
    event_type: GuardrailEventHooks | None,
) -> dict[str, JsonValue] | None:
    if hook_name == "apply_guardrail":
        inputs = kwargs.get("inputs")
        match inputs:
            case dict():
                return copy.deepcopy(inputs)
            case str() | bool() | int() | float() | list() | None:
                return None
    if event_type != GuardrailEventHooks.pre_call:
        return None
    baseline: dict[str, JsonValue] = {}
    for key, value in request_data.items():
        if key in PRE_CALL_CONTENT_KEYS:
            baseline[key] = copy.deepcopy(value)
    return baseline


def infer_event_type(hook_name: str) -> GuardrailEventHooks | None:
    match hook_name:
        case "async_pre_call_hook":
            return GuardrailEventHooks.pre_call
        case "async_moderation_hook":
            return GuardrailEventHooks.during_call
        case "async_post_call_success_hook" | "async_post_call_streaming_hook":
            return GuardrailEventHooks.post_call
        case _:
            return None


def session_text(value: JsonValue) -> str | None:
    match value:
        case str():
            return value
        case bool() | int() | float():
            return str(value)
        case list() | dict() | None:
            return None


def session_id_from_request_data(request_data: Mapping[str, JsonValue]) -> str | None:
    found = session_text(request_data.get("litellm_session_id"))
    if found is not None:
        return found
    for bucket in ("metadata", "litellm_metadata"):
        metadata = request_data.get(bucket)
        match metadata:
            case dict():
                nested = session_text(metadata.get("session_id"))
                if nested is not None:
                    return nested
            case str() | bool() | int() | float() | list() | None:
                continue
    return None


def truthy_meta(value: JsonValue, fallback: dict[str, JsonValue]) -> dict[str, JsonValue]:
    found = fallback
    match value:
        case dict():
            if value:
                found = value
        case str() | bool() | int() | float() | list() | None:
            pass
    return found


def admin_metadata(data: dict[str, JsonValue]) -> dict[str, JsonValue]:
    team_meta: dict[str, JsonValue] = {}
    key_meta: dict[str, JsonValue] = {}
    for bucket_name in ("metadata", "litellm_metadata"):
        bucket = data.get(bucket_name)
        match bucket:
            case dict():
                team_meta = truthy_meta(bucket.get("user_api_key_team_metadata"), team_meta)
                key_meta = truthy_meta(bucket.get("user_api_key_metadata"), key_meta)
            case str() | bool() | int() | float() | list() | None:
                continue
    merged: dict[str, JsonValue] = {**team_meta, **key_meta}
    return merged


def requested_list(value: JsonValue) -> list[JsonValue]:
    match value:
        case list():
            return value
        case str() | bool() | int() | float() | dict() | None:
            raise TypeError("guardrails must be a list")


def is_guardrail_intervention(error: Exception) -> bool:
    match error:
        case (
            ModifyResponseException()
            | GuardrailRaisedException()
            | BlockedPiiEntityError()
            | SensitiveDataRouteException()
        ):
            return True
        case Exception():
            return False
