from pydantic import BaseModel, JsonValue
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


# ast-grep-ignore: no-long-names
class HideSecretsGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    detect_secrets_config: dict[str, JsonValue] | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Hide Secrets"
