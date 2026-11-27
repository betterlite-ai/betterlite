from collections.abc import Awaitable
from typing import Literal

from pydantic import JsonValue
from typing_extensions import Self

from betterlite._integrations.custom_guardrail_constants import (
    LOGS_GUARDRAIL_INFORMATION_MARKER as MARKER,
)
from betterlite._integrations.custom_guardrail_hooks import ApplyGuardrailHook, GuardrailHooks
from betterlite._integrations.custom_logger_types import LiteLLMLoggingObj
from betterlite._integrations.guardrail_types import GenericGuardrailAPIInputs


class GuardrailHookBinding:
    def __init__(self, hook: ApplyGuardrailHook[GuardrailHooks], owner: GuardrailHooks) -> None:
        self.hook, self.owner = hook, owner

    def __call__(
        self, inputs: GenericGuardrailAPIInputs, request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"], logging_obj: LiteLLMLoggingObj | None = None,
    ) -> Awaitable[GenericGuardrailAPIInputs]:
        return self.owner.guarded_apply_guardrail(
            self.hook, inputs, request_data, input_type, logging_obj
        )


class GuardrailHook:
    def __init__(self, hook: ApplyGuardrailHook[GuardrailHooks]) -> None:
        self.hook = hook

    def __call__(
        self, owner: GuardrailHooks, inputs: GenericGuardrailAPIInputs,
        request_data: dict[str, JsonValue], input_type: Literal["request", "response"],
        logging_obj: LiteLLMLoggingObj | None = None,
    ) -> Awaitable[GenericGuardrailAPIInputs]:
        return self.hook(owner, inputs, request_data, input_type, logging_obj)

    def __get__(
        self, instance: GuardrailHooks | None, owner: type[GuardrailHooks] | None = None
    ) -> Self | GuardrailHookBinding:
        _ = owner
        if instance is None:
            return self
        return GuardrailHookBinding(self.hook, instance)


def log_guardrail_information(func: ApplyGuardrailHook[GuardrailHooks]) -> GuardrailHook:
    descriptor = GuardrailHook(func)
    # ast-grep-ignore: no-reflection
    setattr(descriptor, MARKER, True)
    # ast-grep-ignore: no-reflection
    setattr(func, MARKER, True)
    return descriptor
