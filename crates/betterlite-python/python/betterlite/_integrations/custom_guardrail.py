from typing import Final

from pydantic import JsonValue
from typing_extensions import override

from betterlite._integrations.custom_guardrail_constants import (
    LOGS_GUARDRAIL_INFORMATION_MARKER as MARKER,
)
from betterlite._integrations.custom_guardrail_hook_logging import log_guardrail_information
from betterlite._integrations.custom_guardrail_hooks import ApplyGuardrailHook, GuardrailHooks
from betterlite._integrations.custom_guardrail_params import GuardrailParams

APPLY_GUARDRAIL_HOOK: Final = "apply_guardrail"


class CustomGuardrail(GuardrailParams):
    @override
    def __init_subclass__(cls, **kwargs: JsonValue) -> None:
        super().__init_subclass__(**kwargs)
        # ast-grep-ignore: no-reflection
        own: ApplyGuardrailHook[GuardrailHooks] = getattr(cls, APPLY_GUARDRAIL_HOOK)
        # ast-grep-ignore: no-reflection
        marked: JsonValue = getattr(own, MARKER, False)
        if own is GuardrailHooks.apply_guardrail or marked is True:
            return
        # ast-grep-ignore: no-reflection
        setattr(cls, APPLY_GUARDRAIL_HOOK, log_guardrail_information(own))
