from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel
from betterlite.types.utils.base import ResponseObject


# ast-grep-ignore: no-long-names
class LassoGuardrailConfigModelOptionalParams(ResponseObject):
    user_id: str | None = None
    conversation_id: str | None = None


class LassoGuardrailConfigModel(
    # ast-grep-ignore: no-long-names
    GuardrailConfigModel[LassoGuardrailConfigModelOptionalParams],
):
    api_key: str | None = None
    api_base: str | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Lasso Guardrail"
