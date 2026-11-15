from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class PromptGuardConfigModel(GuardrailConfigModel[BaseModel]):
    api_key: str | None = None
    api_base: str | None = None
    block_on_error: bool | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "PromptGuard"
