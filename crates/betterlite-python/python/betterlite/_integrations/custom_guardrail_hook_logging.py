from collections.abc import Awaitable
from typing import Generic, Literal, Self, TypeVar

from pydantic import JsonValue

from betterlite._integrations.custom_guardrail_constants import (
    LOGS_GUARDRAIL_INFORMATION_MARKER as MARKER,
)
from betterlite._integrations.custom_guardrail_hooks import ApplyGuardrailHook, GuardrailHooks
from betterlite._integrations.custom_logger_types import LiteLLMLoggingObj
from betterlite._integrations.guardrail_types import GenericGuardrailAPIInputs

_Owner = TypeVar("_Owner", bound=GuardrailHooks)


class GuardrailHookBinding(Generic[_Owner]):
    def __init__(self, hook: ApplyGuardrailHook[_Owner], owner: _Owner) -> None:
        self.hook, self.owner = hook, owner

    def __call__(
        self, inputs: GenericGuardrailAPIInputs, request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"], logging_obj: LiteLLMLoggingObj | None = None,
    ) -> Awaitable[GenericGuardrailAPIInputs]:
        return self.owner.guarded_apply_guardrail(
            self.owner, self.hook, inputs, request_data, input_type, logging_obj
        )


class GuardrailHook(Generic[_Owner]):
    def __init__(self, hook: ApplyGuardrailHook[_Owner]) -> None:
        self.hook = hook

    def __call__(
        self, owner: _Owner, inputs: GenericGuardrailAPIInputs, request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"], logging_obj: LiteLLMLoggingObj | None = None,
    ) -> Awaitable[GenericGuardrailAPIInputs]:
        return self.hook(owner, inputs, request_data, input_type, logging_obj)

    def __get__(
        self, instance: _Owner | None, owner: type[_Owner] | None = None
    ) -> Self | GuardrailHookBinding[_Owner]:
        _ = owner
        if instance is None:
            return self
        return GuardrailHookBinding(self.hook, instance)


def log_guardrail_information(func: ApplyGuardrailHook[_Owner]) -> GuardrailHook[_Owner]:
    descriptor: GuardrailHook[_Owner] = GuardrailHook(func)
    # ast-grep-ignore: no-reflection
    setattr(descriptor, MARKER, True)
    # ast-grep-ignore: no-reflection
    setattr(func, MARKER, True)
    return descriptor
