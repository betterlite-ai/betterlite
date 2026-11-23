import inspect
import json
from collections.abc import Awaitable, Mapping
from datetime import datetime
from typing import NoReturn

from pydantic import JsonValue
from typing_extensions import TypeIs

from betterlite._integrations.custom_guardrail_constants import STREAM_CONTROL_KEYS
from betterlite._integrations.custom_guardrail_request import (
    apply_log_target,
    apply_records,
    infer_event_type,
    is_guardrail_intervention,
    log_target_json,
    original_inputs_for,
    request_data_of,
    sync_guardrail_info,
)
from betterlite._integrations.custom_guardrail_selection import GuardrailSelection
from betterlite._integrations.custom_guardrail_support import (
    guardrail_summary,
    json_projection,
    record_add,
    settings_of,
)
from betterlite._integrations.custom_logger import LOGGER
from betterlite._integrations.guardrail_types import (
    JSON,
    AsyncHookCallable,
    GuardrailStatus,
    GuardrailTracingDetail,
    GuardrailValue,
    HookCallable,
    RecordContext,
    RecordInput,
    SyncHookCallable,
)
from betterlite.types.guardrails.params import GuardrailEventHooks


def hook_logging_obj(
    kwargs: Mapping[str, JsonValue], request_data: Mapping[str, JsonValue]
) -> JsonValue:
    value = kwargs.get("logging_obj")
    match value:
        case None:
            nested = request_data.get("litellm_logging_obj")
            return nested
        case str() | bool() | int() | float() | list() | dict():
            return value


class GuardrailRecording(GuardrailSelection):
    @staticmethod
    def _is_guardrail_intervention(e: Exception) -> bool:
        return is_guardrail_intervention(e)

    # ast-grep-ignore: no-long-names
    def add_standard_logging_guardrail_information_to_request_data(
        self,
        guardrail_json_response: GuardrailValue,
        request_data: dict[str, JsonValue],
        guardrail_status: GuardrailStatus,
        *,
        start_time: float | None = None,
        end_time: float | None = None,
        duration: float | None = None,
        masked_entity_count: dict[str, int] | None = None,
        guardrail_provider: str | None = None,
        event_type: GuardrailEventHooks | None = None,
        tracing_detail: GuardrailTracingDetail | None = None,
    ) -> None:
        detail = None if tracing_detail is None else JSON.validate_json(json.dumps(tracing_detail))
        record = RecordInput(
            guardrail_response=json_projection(guardrail_json_response),
            guardrail_status=guardrail_status,
            start_time=start_time,
            end_time=end_time,
            duration=duration,
            masked_entity_count=masked_entity_count,
            guardrail_provider=guardrail_provider,
            event_type=None if event_type is None else str(event_type),
            tracing_detail=detail,
        )
        logging_obj = request_data.get("litellm_logging_obj")
        context = RecordContext(
            settings=settings_of(self),
            log_target=log_target_json(logging_obj),
        )
        updated, target = record_add(request_data, record, context)
        apply_records(request_data, updated)
        apply_log_target(logging_obj, target)
        self._guardrail_self_recorded = True

    def _process_response(
        self,
        response: GuardrailValue,
        request_data: dict[str, JsonValue],
        *,
        start_time: float | None = None,
        end_time: float | None = None,
        duration: float | None = None,
        event_type: GuardrailEventHooks | None = None,
        original_inputs: dict[str, JsonValue] | None = None,
    ) -> GuardrailValue:
        guardrail_response = self._summarize_guardrail_response(
            response, original_inputs, event_type
        )
        LOGGER.debug("Guardrail response: %s", guardrail_response)
        self.add_standard_logging_guardrail_information_to_request_data(
            guardrail_response,
            request_data,
            "success",
            duration=duration,
            start_time=start_time,
            end_time=end_time,
            event_type=event_type,
        )
        return response

    def _summarize_guardrail_response(
        self,
        response: GuardrailValue,
        original_inputs: Mapping[str, JsonValue] | None,
        event_type: GuardrailEventHooks | None,
    ) -> JsonValue:
        return guardrail_summary(response, original_inputs, event_type)

    def _process_error(
        self,
        e: Exception,
        request_data: dict[str, JsonValue],
        *,
        start_time: float | None = None,
        end_time: float | None = None,
        duration: float | None = None,
        event_type: GuardrailEventHooks | None = None,
    ) -> NoReturn:
        status: GuardrailStatus = (
            "guardrail_intervened"
            if self._is_guardrail_intervention(e)
            else "guardrail_failed_to_respond"
        )
        response = "deny" if "CustomCodeGuardrail" in e.__class__.__name__ else str(e)
        self.add_standard_logging_guardrail_information_to_request_data(
            response,
            request_data,
            status,
            duration=duration,
            start_time=start_time,
            end_time=end_time,
            event_type=event_type,
        )
        raise e

    def _inputs_were_modified(
        self, original_inputs: Mapping[str, JsonValue], response: Mapping[str, JsonValue]
    ) -> bool:
        compared = (original_inputs.keys() | response.keys()) - STREAM_CONTROL_KEYS
        return any(original_inputs.get(key) != response.get(key) for key in compared)


class _GuardrailHook(GuardrailRecording):
    # ast-grep-ignore: no-long-names
    _litellm_logs_guardrail_information: bool = True
    hook: HookCallable
    hook_name: str

    def __init__(self, hook: HookCallable, hook_name: str) -> None:
        self.hook = hook
        self.hook_name = hook_name

    def __get__(
        self, instance: GuardrailRecording | None, owner: type[GuardrailRecording] | None = None
    ) -> HookCallable:
        _ = owner
        bound: HookCallable = _GuardrailHookBinding(self, instance)
        return bound

    def run(
        self,
        owner: GuardrailRecording | None,
        args: tuple[JsonValue, ...],
        kwargs: dict[str, JsonValue],
    ) -> GuardrailValue | Awaitable[GuardrailValue]:
        hook = self.hook
        if is_async_hook(hook):
            pending: Awaitable[GuardrailValue] = self.run_async(owner, hook, args, kwargs)
            return pending
        if is_sync_hook(hook):
            return self.run_sync(owner, hook, args, kwargs)
        raise TypeError(f"{self.hook_name} is not a callable guardrail hook")

    async def run_async(
        self,
        owner: GuardrailRecording | None,
        hook: AsyncHookCallable,
        args: tuple[JsonValue, ...],
        kwargs: dict[str, JsonValue],
    ) -> GuardrailValue:
        if owner is None:
            return await hook(*args, **kwargs)
        return await self._run_async_owner(owner, hook, args, kwargs, start=datetime.now())

    async def _run_async_owner(
        self,
        owner: GuardrailRecording,
        hook: AsyncHookCallable,
        args: tuple[JsonValue, ...],
        kwargs: dict[str, JsonValue],
        *,
        start: datetime,
    ) -> GuardrailValue:
        request_data = request_data_of(kwargs)
        event_type = infer_event_type(self.hook_name)
        original_inputs = original_inputs_for(self.hook_name, kwargs, request_data, event_type)
        logging_obj = hook_logging_obj(kwargs, request_data)
        owner._guardrail_self_recorded = False
        try:
            response = await hook(owner, *args, **kwargs)
            if owner.records_own_guardrail_information or owner._guardrail_self_recorded:
                return response
            return owner._process_response(
                response,
                request_data,
                start_time=start.timestamp(),
                end_time=datetime.now().timestamp(),
                duration=(datetime.now() - start).total_seconds(),
                event_type=event_type,
                original_inputs=original_inputs,
            )
        except Exception as error:
            LOGGER.debug("Guardrail %s raised", self.hook_name, exc_info=True)
            if owner._guardrail_self_recorded:
                raise
            return owner._process_error(
                error,
                request_data,
                start_time=start.timestamp(),
                end_time=datetime.now().timestamp(),
                duration=(datetime.now() - start).total_seconds(),
                event_type=event_type,
            )
        finally:
            owner._guardrail_self_recorded = False
            sync_guardrail_info(request_data, logging_obj)

    def run_sync(
        self,
        owner: GuardrailRecording | None,
        hook: SyncHookCallable,
        args: tuple[JsonValue, ...],
        kwargs: dict[str, JsonValue],
    ) -> GuardrailValue:
        if owner is None:
            return hook(*args, **kwargs)
        return self._run_sync_owner(owner, hook, args, kwargs, start=datetime.now())

    def _run_sync_owner(
        self,
        owner: GuardrailRecording,
        hook: SyncHookCallable,
        args: tuple[JsonValue, ...],
        kwargs: dict[str, JsonValue],
        *,
        start: datetime,
    ) -> GuardrailValue:
        request_data = request_data_of(kwargs)
        event_type = infer_event_type(self.hook_name)
        original_inputs = original_inputs_for(self.hook_name, kwargs, request_data, event_type)
        logging_obj = hook_logging_obj(kwargs, request_data)
        owner._guardrail_self_recorded = False
        try:
            response = hook(owner, *args, **kwargs)
            if owner.records_own_guardrail_information or owner._guardrail_self_recorded:
                return response
            return owner._process_response(
                response,
                request_data,
                duration=(datetime.now() - start).total_seconds(),
                event_type=event_type,
                original_inputs=original_inputs,
            )
        except Exception as error:
            LOGGER.debug("Guardrail %s raised", self.hook_name, exc_info=True)
            if owner._guardrail_self_recorded:
                raise
            return owner._process_error(
                error,
                request_data,
                duration=(datetime.now() - start).total_seconds(),
                event_type=event_type,
            )
        finally:
            owner._guardrail_self_recorded = False
            sync_guardrail_info(request_data, logging_obj)


class _GuardrailHookBinding:
    hook: _GuardrailHook
    owner: GuardrailRecording | None

    def __init__(self, hook: _GuardrailHook, owner: GuardrailRecording | None) -> None:
        self.hook = hook
        self.owner = owner

    def __call__(
        self, *args: JsonValue, **kwargs: JsonValue
    ) -> GuardrailValue | Awaitable[GuardrailValue]:
        return self.hook.run(self.owner, args, kwargs)


def is_async_hook(value: HookCallable) -> TypeIs[AsyncHookCallable]:
    if inspect.iscoroutinefunction(value):
        return True
    return False


def is_sync_hook(value: HookCallable) -> TypeIs[SyncHookCallable]:
    if inspect.iscoroutinefunction(value):
        return False
    return True


def is_guarded_hook(value: HookCallable | _GuardrailHook) -> TypeIs[_GuardrailHook]:
    match value:
        case _GuardrailHook():
            return True
        case object():
            return False


def log_guardrail_information(func: HookCallable) -> HookCallable:
    # ast-grep-ignore: no-reflection
    name: str = getattr(func, "__name__", "")
    hooked: HookCallable = _GuardrailHook(func, name)
    return hooked
