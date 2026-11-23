from pydantic import JsonValue

from betterlite._integrations.custom_guardrail_hook_logging import (
    is_guarded_hook,
    log_guardrail_information,
)
from betterlite._integrations.custom_guardrail_params import GuardrailParams
from betterlite._integrations.guardrail_types import HookCallable


class CustomGuardrail(GuardrailParams):
    def __init_subclass__(cls, **kwargs: JsonValue) -> None:
        _ = kwargs
        # ast-grep-ignore: no-reflection
        own: HookCallable | None = cls.__dict__.get("apply_guardrail")
        if own is None or is_guarded_hook(own):
            return
        hooked = log_guardrail_information(own)
        # ast-grep-ignore: no-reflection
        setattr(cls, "apply_guardrail", hooked)
