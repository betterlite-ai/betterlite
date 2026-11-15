from typing import Literal

from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class HeadroomGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    api_base: str | None = None
    api_key: str | None = None
    model: str | None = None
    unreachable_fallback: Literal["fail_closed", "fail_open"] = "fail_closed"
    ccr_retrieval: bool = True

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Headroom"
