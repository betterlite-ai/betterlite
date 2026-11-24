from collections.abc import Mapping

from pydantic import JsonValue

from betterlite._integrations.custom_guardrail_constants import RECORDS_KEY
from betterlite._integrations.custom_guardrail_support import (
    json_projection,
    request_json,
    synced_target,
)
from betterlite._integrations.custom_logger_types import LiteLLMLoggingObj
from betterlite._integrations.guardrail_types import (
    BUCKET,
    OBJECT,
    REQUEST,
    AdminMetadata,
    LogTarget,
)
from betterlite.exceptions._process import (
    BlockedPiiEntityError,
    GuardrailRaisedException,
    ModifyResponseException,
    SensitiveDataRouteException,
)


def project_mapping(value: JsonValue) -> dict[str, JsonValue] | None:
    match value:
        case dict():
            return {name: json_projection(item) for name, item in value.items()}
        case str() | bool() | int() | float() | list() | None:
            return None


def live_views(logging_obj: LiteLLMLoggingObj) -> tuple[JsonValue, JsonValue]:
    # ast-grep-ignore: no-reflection
    live: JsonValue = getattr(logging_obj, "litellm_params", None)
    # ast-grep-ignore: no-reflection
    details: dict[str, JsonValue] | None = getattr(logging_obj, "model_call_details", None)
    return live, None if details is None else details.get("litellm_params")


def log_target_json(logging_obj: LiteLLMLoggingObj | None) -> LogTarget | None:
    if logging_obj is None:
        return None
    live, nested = live_views(logging_obj)
    return LogTarget(
        litellm_params=project_mapping(live),
        model_call_params=project_mapping(nested),
    )


def apply_log_target(logging_obj: LiteLLMLoggingObj | None, target: LogTarget | None) -> None:
    if logging_obj is None or target is None:
        return
    live, nested = live_views(logging_obj)
    for view, params in ((live, target["litellm_params"]), (nested, target["model_call_params"])):
        metadata = None if params is None else params.get("metadata")
        if metadata is None:
            continue
        match view:
            case dict():
                view["metadata"] = metadata
            case str() | bool() | int() | float() | list() | None:
                pass


def apply_records(request_data: dict[str, JsonValue], updated: dict[str, JsonValue]) -> None:
    bucket_name = "litellm_metadata" if "litellm_metadata" in request_data else "metadata"
    bucket = OBJECT.validate_python(updated.get(bucket_name))
    entries = bucket.get(RECORDS_KEY)
    live = BUCKET.validate_python(request_data.get(bucket_name))
    if live is None:
        request_data[bucket_name] = bucket
        return
    live[RECORDS_KEY] = entries
    request_data[bucket_name] = live


def sync_guardrail_info(
    request_data: dict[str, JsonValue], logging_obj: LiteLLMLoggingObj | None
) -> None:
    target = log_target_json(logging_obj)
    if target is None:
        return
    apply_log_target(logging_obj, synced_target(request_data, target))


def session_id_from_request_data(request_data: Mapping[str, JsonValue]) -> str | None:
    data = REQUEST.validate_python(request_json(request_data))
    direct = data.get("litellm_session_id")
    if direct is not None:
        return str(direct)
    for bucket in (data.get("metadata"), data.get("litellm_metadata")):
        session = None if bucket is None else bucket.get("session_id")
        if session is not None:
            return str(session)
    return None


def admin_metadata(data: Mapping[str, JsonValue]) -> AdminMetadata:
    view = REQUEST.validate_python(request_json(data))
    team: AdminMetadata = {}
    key: AdminMetadata = {}
    for bucket in (view.get("metadata"), view.get("litellm_metadata")):
        if bucket is None:
            continue
        team = bucket.get("user_api_key_team_metadata") or team
        key = bucket.get("user_api_key_metadata") or key
    merged: AdminMetadata = {}
    merged.update(team)
    merged.update(key)
    return merged


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
