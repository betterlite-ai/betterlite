from enum import Enum

from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class HiddenlayerAction(str, Enum):
    BLOCK = "Block"
    REDACT = "Redact"


class HiddenlayerMessages(str, Enum):
    BLOCK_MESSAGE = "Blocked by Hiddenlayer."


# ast-grep-ignore: no-long-names
class HiddenlayerGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    api_base: str | None = None
    api_id: str | None = None
    api_key: str | None = None
    version: int | None = 2

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Hiddenlayer Guardrail"
