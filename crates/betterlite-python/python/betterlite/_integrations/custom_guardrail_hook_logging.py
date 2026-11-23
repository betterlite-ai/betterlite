import inspect
from collections.abc import Awaitable
from datetime import datetime

from pydantic import JsonValue
from typing_extensions import TypeIs

from betterlite._integrations.custom_guardrail_recording import (
    GuardrailRecording,
    hook_logging_obj,
)
from betterlite._integrations.custom_guardrail_request import (
    infer_event_type,
    original_inputs_for,
    request_data_of,
    sync_guardrail_info,
)
from betterlite._integrations.custom_logger import LOGGER
from betterlite._integrations.guardrail_types import (
    AsyncHookCallable,
    GuardrailValue,
    HookCallable,
    SyncHookCallable,
)


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
