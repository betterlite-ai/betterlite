import json
import os
from collections.abc import Mapping
from typing import Final, NoReturn

from pydantic import JsonValue

from betterlite._integrations.custom_guardrail_constants import TAG_MODE_MESSAGE
from betterlite._integrations.guardrail_types import (
    ERROR_PAYLOAD,
    JSON,
    LOG_TARGET,
    OBJECT,
    SETTINGS,
    GuardrailSettingsOwner,
    GuardrailValue,
    LogTarget,
    RecordContext,
    RecordInput,
    SettingsJson,
    SwitchesJson,
)
from betterlite._native import (
    NativeGuardrail,
    NativeGuardrailError,
    add_guardrail_record,
    messages_for_call_type,
    summarize_guardrail_response,
    sync_log_target,
)
from betterlite.types.guardrails.params import GuardrailEventHooks, Mode
from betterlite.types.utils.base import Dumpable

STRICT_MODES_VARIABLE: Final = "LITELLM_STRICT_GUARDRAIL_MODES"
_ERROR_TYPES: Final[dict[str, type[Exception]]] = {
    "AlreadyFinished": RuntimeError,
    "CacheUnavailable": ValueError,
    "ContextValueNotText": ValueError,
    "DynamicParamsNotObject": TypeError,
    "InvalidEventType": ValueError,
    "InvalidJson": ValueError,
    "Json": ValueError,
    "MessagesShape": ValueError,
    "RequestedNotList": TypeError,
    "ResponsesInput": ValueError,
    "UnknownEventHook": ValueError,
    "UnsupportedEventHook": ValueError,
}


def strict_guardrail_modes() -> bool:
    raw = os.environ.get(STRICT_MODES_VARIABLE)
    if raw is None:
        return True
    match raw.strip().lower():
        case "true":
            return True
        case "false":
            return False
        case _:
            return True


def json_projection(value: GuardrailValue) -> JsonValue:
    match value:
        case None | bool() | int() | float() | str():
            return JSON.validate_python(value)
        case dict():
            return {name: json_projection(item) for name, item in value.items()}
        case list():
            return [json_projection(item) for item in value]
        case Dumpable():
            return JSON.validate_python(value.model_dump(mode="json"))
        case object():
            return str(value)


def hook_json(
    value: GuardrailEventHooks | list[GuardrailEventHooks] | Mode | None,
) -> JsonValue:
    match value:
        case None:
            return None
        case Mode():
            return JSON.validate_python(value.model_dump(mode="json"))
        case list():
            return JSON.validate_python([str(item) for item in value])
        case GuardrailEventHooks():
            return str(value)
        case object():
            return str(value)


def settings_of(owner: GuardrailSettingsOwner) -> SettingsJson:
    supported = owner.supported_event_hooks
    hooks: list[str] | None = None if supported is None else [str(hook) for hook in supported]
    switches = SwitchesJson(
        use_native_during_call_hook=owner.use_native_during_call_hook,
        use_native_lifecycle_hooks=owner.use_native_lifecycle_hooks,
        records_own_information=owner.records_own_guardrail_information,
        uses_apply_guardrail_interface=owner.uses_apply_guardrail_interface(),
        deny_text_on_error="CustomCodeGuardrail" in owner.__class__.__name__,
    )
    return SettingsJson(
        guardrail_name=owner.guardrail_name,
        supported_event_hooks=hooks,
        event_hook=hook_json(owner.event_hook),
        default_on=owner.default_on,
        mask_request_content=owner.mask_request_content,
        mask_response_content=owner.mask_response_content,
        violation_message_template=owner.violation_message_template,
        end_session_after_n_fails=owner.end_session_after_n_fails,
        on_violation=owner.on_violation,
        realtime_violation_message=owner.realtime_violation_message,
        on_sensitive_data=owner.on_sensitive_data,
        sensitive_data_route_to_model=owner.sensitive_data_route_to_model,
        sticky_session_routing=owner.sticky_session_routing,
        run_in_parallel=owner.run_in_parallel,
        scan_raw_request=owner.scan_raw_request,
        only_scan_new_messages=owner.only_scan_new_messages,
        timeout=owner.timeout,
        switches=switches,
    )


def translate_native_error(error: NativeGuardrailError) -> Exception:
    payload = ERROR_PAYLOAD.validate_json(str(error))
    kind = payload["kind"]
    if kind == "TagMode":
        return ImportError(TAG_MODE_MESSAGE)
    found = _ERROR_TYPES.get(kind)
    if found is None:
        raise RuntimeError(payload["message"]) from error
    return found(payload["message"])


def raise_native_error(error: NativeGuardrailError) -> NoReturn:
    raise translate_native_error(error) from error


def native_handle(owner: GuardrailSettingsOwner, *, strict: bool) -> NativeGuardrail:
    try:
        return NativeGuardrail(json.dumps(settings_of(owner)), strict)
    except NativeGuardrailError as error:
        raise_native_error(error)


def selection_answer(
    handle: NativeGuardrail,
    data: Mapping[str, JsonValue],
    event_type: GuardrailEventHooks,
    suppressed: frozenset[str],
) -> bool:
    data_text = json.dumps(json_projection(dict(data)))
    names = json.dumps(sorted(suppressed))
    try:
        return handle.should_run_guardrail(data_text, str(event_type), names)
    except NativeGuardrailError as error:
        raise_native_error(error)


def event_hook_matches(handle: NativeGuardrail, event_type: GuardrailEventHooks) -> bool:
    try:
        return handle.event_hook_is_event_type(str(event_type))
    except NativeGuardrailError as error:
        raise_native_error(error)


def dynamic_params(
    handle: NativeGuardrail, request_data: Mapping[str, JsonValue]
) -> dict[str, JsonValue]:
    try:
        text = handle.dynamic_request_body_params(json.dumps(json_projection(dict(request_data))))
    except NativeGuardrailError as error:
        raise_native_error(error)
    return OBJECT.validate_json(text)


def render_message(handle: NativeGuardrail, default: str, context: Mapping[str, str]) -> str:
    try:
        return handle.render_violation_message(default, json.dumps(dict(context)))
    except NativeGuardrailError as error:
        raise_native_error(error)


def advisory_message(
    data: Mapping[str, JsonValue], message: str
) -> tuple[bool, dict[str, JsonValue]]:
    try:
        written, updated = NativeGuardrail.inject_advisory_message(
            json.dumps(json_projection(dict(data))), message
        )
    except NativeGuardrailError as error:
        raise_native_error(error)
    return written, OBJECT.validate_json(updated)


def update_native_params(handle: NativeGuardrail, params: Mapping[str, JsonValue]) -> None:
    try:
        handle.update_params(json.dumps(dict(params)))
    except NativeGuardrailError as error:
        raise_native_error(error)


def native_settings(handle: NativeGuardrail) -> SettingsJson:
    try:
        text = handle.settings()
    except NativeGuardrailError as error:
        raise_native_error(error)
    return SETTINGS.validate_json(text)


def record_add(
    data: Mapping[str, JsonValue], record: RecordInput, context: RecordContext
) -> tuple[dict[str, JsonValue], LogTarget | None]:
    try:
        updated, target = add_guardrail_record(
            json.dumps(json_projection(dict(data))), json.dumps(record), json.dumps(context)
        )
    except NativeGuardrailError as error:
        raise_native_error(error)
    return OBJECT.validate_json(updated), LOG_TARGET.validate_json(target)


def guardrail_summary(
    response: GuardrailValue,
    baseline: Mapping[str, JsonValue] | None,
    event_type: GuardrailEventHooks | None,
) -> JsonValue:
    baseline_text = None if baseline is None else json.dumps(dict(baseline))
    event_text = None if event_type is None else str(event_type)
    try:
        text = summarize_guardrail_response(
            json.dumps(json_projection(response)), baseline_text, event_text
        )
    except NativeGuardrailError as error:
        raise_native_error(error)
    return JSON.validate_json(text)


def call_type_messages(call_type: str, data: Mapping[str, JsonValue]) -> JsonValue:
    try:
        text = messages_for_call_type(call_type, json.dumps(json_projection(dict(data))))
    except NativeGuardrailError as error:
        raise_native_error(error)
    if text is None:
        return None
    return JSON.validate_json(text)


def synced_target(
    data: Mapping[str, JsonValue], target: LogTarget
) -> LogTarget | None:
    try:
        text = sync_log_target(json.dumps(json_projection(dict(data))), json.dumps(target))
    except NativeGuardrailError as error:
        raise_native_error(error)
    return LOG_TARGET.validate_json(text)
