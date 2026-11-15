from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel
from betterlite.types.utils.base import ResponseObject


# ast-grep-ignore: no-long-names
class PangeaGuardrailConfigModelOptionalParams(ResponseObject):
    pangea_input_recipe: str | None = None
    pangea_output_recipe: str | None = None


class PangeaGuardrailConfigModel(
    # ast-grep-ignore: no-long-names
    GuardrailConfigModel[PangeaGuardrailConfigModelOptionalParams],
):
    api_key: str | None = None
    api_base: str | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Pangea Guardrail"
