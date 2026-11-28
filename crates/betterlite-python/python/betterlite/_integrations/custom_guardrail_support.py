import json
import os
from collections.abc import Callable, Mapping
from functools import partial
from typing import Final, TypeVar

from pydantic import JsonValue

from betterlite._integrations.custom_guardrail_constants import TAG_MODE_MESSAGE
from betterlite._integrations.guardrail_types import (
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
    "AlreadyFinished": RuntimeError, "DynamicParamsNotObject": TypeError,
    "RequestedNotList": TypeError, "CacheUnavailable": ValueError,
    "ContextValueNotText": ValueError, "InvalidEventType": ValueError,
    "InvalidJson": ValueError, "Json": ValueError, "MessagesShape": ValueError,
    "ResponsesInput": ValueError, "UnknownEventHook": ValueError,
    "UnsupportedEventHook": ValueError,
}
_Result = TypeVar("_Result")


def guarded(operation: Callable[[], _Result]) -> _Result:
    try:
        return operation()
    except NativeGuardrailError as error:
        raise translate_native_error(error) from error


def translate_native_error(error: NativeGuardrailError) -> Exception:
    payload = OBJECT.validate_json(str(error))
    kind = text_value(payload.get("kind"), "")
    message = text_value(payload.get("message"), "")
    if kind == "TagMode":
        return ImportError(TAG_MODE_MESSAGE)
    found = _ERROR_TYPES.get(kind)
    if found is None:
        raise RuntimeError(message) from error
    return found(message)


def strict_guardrail_modes() -> bool:
    raw = os.environ.get(STRICT_MODES_VARIABLE)
    if raw is None:
        return True
    return raw.strip().lower() != "false"


def text_value(value: JsonValue, fallback: str) -> str:
    match value:
        case str():
            return value
        case bool() | int() | float() | list() | dict() | None:
            return fallback


def json_projection(value: GuardrailValue) -> JsonValue:
    match value:
        case Exception():
            return str(value)
        case Dumpable():
            return JSON.validate_python(value.model_dump(mode="json"))
        case dict() | list() | bool() | int() | float() | str() | None:
            return JSON.validate_python(value)


def request_json(data: Mapping[str, JsonValue]) -> dict[str, JsonValue]:
    kept = {name: value for name, value in data.items() if name != "litellm_logging_obj"}
    return OBJECT.validate_python(kept)


def settings_of(owner: GuardrailSettingsOwner) -> SettingsJson:
    supported = owner.supported_event_hooks
    hooks: list[str] | None = None if supported is None else [hook.value for hook in supported]
    requested = owner.event_hook
    event_hook: JsonValue = None
    match requested:
        case Mode():
            event_hook = JSON.validate_python(requested.model_dump(mode="json"))
        case list():
            event_hook = JSON.validate_json(json.dumps(requested))
        case GuardrailEventHooks() | None:
            event_hook = None if requested is None else requested.value
        case str():
            event_hook = requested
    switches = SwitchesJson(
        use_native_during_call_hook=owner.use_native_during_call_hook is True,
        use_native_lifecycle_hooks=owner.use_native_lifecycle_hooks is True,
        records_own_information=owner.records_own_guardrail_information is True,
        uses_apply_guardrail_interface=owner.uses_apply_guardrail_interface(),
        deny_text_on_error="CustomCodeGuardrail" in owner.__class__.__name__,
    )
    return SettingsJson(
        guardrail_name=owner.guardrail_name, supported_event_hooks=hooks,
        event_hook=event_hook, default_on=owner.default_on is True,
        mask_request_content=owner.mask_request_content is True,
        mask_response_content=owner.mask_response_content is True,
        violation_message_template=owner.violation_message_template,
        end_session_after_n_fails=owner.end_session_after_n_fails,
        on_violation=owner.on_violation,
        realtime_violation_message=owner.realtime_violation_message,
        on_sensitive_data=owner.on_sensitive_data,
        sensitive_data_route_to_model=owner.sensitive_data_route_to_model,
        sticky_session_routing=owner.sticky_session_routing is not False,
        run_in_parallel=owner.run_in_parallel is True,
        scan_raw_request=owner.scan_raw_request is True,
        only_scan_new_messages=owner.only_scan_new_messages is True,
        timeout=owner.timeout, switches=switches,
    )


def native_handle(owner: GuardrailSettingsOwner, *, strict: bool) -> NativeGuardrail:
    text = json.dumps(settings_of(owner))
    return guarded(partial(NativeGuardrail, text, strict_modes=strict))


def selection_answer(
    handle: NativeGuardrail, data: Mapping[str, JsonValue],
    event_type: GuardrailEventHooks, suppressed: frozenset[str],
) -> bool:
    ordered: list[str] = sorted(suppressed)
    names = json.dumps(ordered)
    request_text = json.dumps(request_json(data))
    return guarded(partial(handle.should_run_guardrail, request_text, event_type.value, names))


def event_hook_matches(handle: NativeGuardrail, event_type: GuardrailEventHooks) -> bool:
    name = event_type.value
    return guarded(partial(handle.event_hook_is_event_type, name))


def dynamic_params(
    handle: NativeGuardrail, request_data: Mapping[str, JsonValue]
) -> dict[str, JsonValue]:
    text = json.dumps(request_json(request_data))
    return OBJECT.validate_json(guarded(partial(handle.dynamic_request_body_params, text)))


def render_message(handle: NativeGuardrail, default: str, context: dict[str, str]) -> str:
    text = json.dumps(context)
    return guarded(partial(handle.render_violation_message, default, text))


def advisory_message(
    data: Mapping[str, JsonValue], message: str
) -> tuple[bool, dict[str, JsonValue]]:
    text = json.dumps(request_json(data))
    written, updated = guarded(partial(NativeGuardrail.inject_advisory_message, text, message))
    return written, OBJECT.validate_json(updated)


def update_native_params(handle: NativeGuardrail, params: dict[str, JsonValue]) -> None:
    payload = json.dumps(params)
    guarded(partial(handle.update_params, payload))


def native_settings(handle: NativeGuardrail) -> SettingsJson:
    text = guarded(handle.settings)
    return SETTINGS.validate_json(text)


def record_add(
    data: Mapping[str, JsonValue], record: RecordInput, context: RecordContext
) -> tuple[dict[str, JsonValue], LogTarget | None]:
    payload = json.dumps(request_json(data))
    call = partial(add_guardrail_record, payload, json.dumps(record), json.dumps(context))
    updated, target = guarded(call)
    return OBJECT.validate_json(updated), LOG_TARGET.validate_json(target)


def guardrail_summary(
    response: GuardrailValue,
    baseline: dict[str, JsonValue] | None,
    event_type: GuardrailEventHooks | None,
) -> JsonValue:
    baseline_text = None if baseline is None else json.dumps(baseline)
    event_text = None if event_type is None else event_type.value
    call = partial(
        summarize_guardrail_response, json.dumps(json_projection(response)),
        baseline_text, event_text,
    )
    return JSON.validate_json(guarded(call))


def call_type_messages(call_type: str, data: Mapping[str, JsonValue]) -> JsonValue:
    text = json.dumps(request_json(data))
    answer = guarded(partial(messages_for_call_type, call_type, text))
    if answer is None:
        return None
    return JSON.validate_json(answer)


def synced_target(data: Mapping[str, JsonValue], target: LogTarget) -> LogTarget | None:
    text = json.dumps(request_json(data))
    return LOG_TARGET.validate_json(guarded(partial(sync_log_target, text, json.dumps(target))))
