from collections.abc import Awaitable
from typing import Literal

from pydantic import JsonValue

from betterlite._integrations.custom_guardrail_constants import (
    LOGS_GUARDRAIL_INFORMATION_MARKER as MARKER,
)
from betterlite._integrations.custom_guardrail_hooks import ApplyGuardrailHook, GuardrailHooks
from betterlite._integrations.custom_logger_types import LiteLLMLoggingObj
from betterlite._integrations.guardrail_types import GenericGuardrailAPIInputs


class GuardrailHookBinding:
    hook: ApplyGuardrailHook
    owner: GuardrailHooks

    def __init__(self, hook: ApplyGuardrailHook, owner: GuardrailHooks) -> None:
        self.hook, self.owner = hook, owner

    def __call__(
        self, inputs: GenericGuardrailAPIInputs, request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"], logging_obj: LiteLLMLoggingObj | None = None,
    ) -> Awaitable[GenericGuardrailAPIInputs]:
        return self.owner.guarded_apply_guardrail(
            self.hook, inputs, request_data, input_type, logging_obj
        )


class GuardrailHook:
    hook: ApplyGuardrailHook

    def __init__(self, hook: ApplyGuardrailHook) -> None:
        self.hook = hook

    def __get__(
        self, instance: GuardrailHooks | None, owner: type[GuardrailHooks] | None = None
    ) -> ApplyGuardrailHook | GuardrailHookBinding:
        _ = owner
        if instance is None:
            return self.hook
        return GuardrailHookBinding(self.hook, instance)


def log_guardrail_information(func: ApplyGuardrailHook) -> GuardrailHook:
    descriptor = GuardrailHook(func)
    enabled = True
    # ast-grep-ignore: no-reflection
    setattr(descriptor, MARKER, enabled)
    # ast-grep-ignore: no-reflection
    setattr(func, MARKER, enabled)
    return descriptor
