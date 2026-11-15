from typing import ClassVar, Final, Literal, Required, TypeAlias

from pydantic import ConfigDict, Field, JsonValue
from typing_extensions import TypedDict, override

from betterlite.types.proxy.guardrails.guardrail_hooks.azure.base import (
    AzureContentSafetyConfigModel,
)
from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel
from betterlite.types.utils.base import ResponseObject

# ast-grep-ignore: no-long-names
AZURE_CONTENT_SAFETY_CATEGORIES: Final = ["Hate", "SelfHarm", "Sexual", "Violence"]


# ast-grep-ignore: no-long-names
class AzureTextModerationRequestBodyOptionalParams(TypedDict, total=False):
    categories: list[str] | None
    blocklistNames: list[str] | None
    haltOnBlocklistHit: bool | None
    outputType: Literal["FourSeverityLevels", "EightSeverityLevels"]


# ast-grep-ignore: no-long-names
class AzureTextModerationGuardrailRequestBody(AzureTextModerationRequestBodyOptionalParams):
    text: Required[str]


# ast-grep-ignore: no-long-names
class AzureTextModerationGuardrailResponseCategoriesAnalysis(TypedDict):
    category: str
    severity: int


# ast-grep-ignore: no-long-names
class AzureTextModerationGuardrailResponse(TypedDict):
    blocklistsMatch: list[dict[str, JsonValue]]
    # ast-grep-ignore: no-long-names
    categoriesAnalysis: list[AzureTextModerationGuardrailResponseCategoriesAnalysis]


AzureHarmCategories: TypeAlias = Literal["Hate", "SelfHarm", "Sexual", "Violence"]


# ast-grep-ignore: no-long-names
class AzureTextModerationOptionalParams(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", serialize_by_alias=True)

    severity_threshold: int | None = None
    severity_threshold_by_category: dict[AzureHarmCategories, int] | None = None
    categories: list[AzureHarmCategories] | None = None
    blocklist_names: list[str] | None = Field(default=None, alias="blocklistNames")
    halt_on_blocklist_hit: bool | None = Field(default=None, alias="haltOnBlocklistHit")
    output_type: Literal["FourSeverityLevels", "EightSeverityLevels"] | None = Field(
        default=None,
        alias="outputType",
    )


# ast-grep-ignore: no-long-names
class AzureContentSafetyTextModerationConfigModel(
    AzureContentSafetyConfigModel,
    # ast-grep-ignore: no-long-names
    GuardrailConfigModel[AzureTextModerationOptionalParams],
):
    # ast-grep-ignore: no-long-names
    optional_params: AzureTextModerationOptionalParams | None = Field()

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Azure Content Safety Text Moderation"
