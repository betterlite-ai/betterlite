from typing import Final

from pydantic import JsonValue
from typing_extensions import override

from betterlite._integrations.custom_guardrail_constants import (
    LOGS_GUARDRAIL_INFORMATION_MARKER as MARKER,
)
from betterlite._integrations.custom_guardrail_hook_logging import log_guardrail_information
from betterlite._integrations.custom_guardrail_hooks import GuardrailHooks
from betterlite._integrations.custom_guardrail_params import GuardrailParams

APPLY_GUARDRAIL_HOOK: Final = "apply_guardrail"


class CustomGuardrail(GuardrailParams):
    @override
    def __init_subclass__(cls, **kwargs: JsonValue) -> None:
        super().__init_subclass__(**kwargs)
        own = cls.apply_guardrail
        # ast-grep-ignore: no-reflection
        marked: JsonValue = getattr(own, MARKER, False)
        if own is GuardrailHooks.apply_guardrail or marked is True:
            return
        # ast-grep-ignore: no-reflection
        setattr(cls, APPLY_GUARDRAIL_HOOK, log_guardrail_information(own))
