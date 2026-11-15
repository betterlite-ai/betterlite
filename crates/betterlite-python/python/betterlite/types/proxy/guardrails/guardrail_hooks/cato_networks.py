from pydantic import BaseModel, Field
from typing_extensions import override

from betterlite.types.guardrails.pii import GuardrailParamUITypes
from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


# ast-grep-ignore: no-long-names
class CatoNetworksGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    api_key: str | None = None
    api_base: str | None = None
    inspect_embeddings: bool | None = Field(
        default=False,
        json_schema_extra={"ui_type": GuardrailParamUITypes.BOOL.value},
    )

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Cato Networks Guardrail"
