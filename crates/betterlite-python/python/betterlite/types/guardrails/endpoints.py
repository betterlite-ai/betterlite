from datetime import datetime

from pydantic import Field, JsonValue

from betterlite.types.guardrails.params import GUARDRAIL_DEFINITION_LOCATION, BaseLitellmParams
from betterlite.types.guardrails.pii import PiiEntityCategoryMap, PiiEntityType
from betterlite.types.utils.base import ResponseObject


class GuardrailInfoResponse(ResponseObject):
    guardrail_id: str | None = None
    guardrail_name: str
    litellm_params: BaseLitellmParams | None = None
    guardrail_info: dict[str, JsonValue] | None = None
    created_at: datetime | None = None
    updated_at: datetime | None = None
    guardrail_definition_location: GUARDRAIL_DEFINITION_LOCATION = (
        GUARDRAIL_DEFINITION_LOCATION.CONFIG
    )


class ListGuardrailsResponse(ResponseObject):
    guardrails: list[GuardrailInfoResponse]


# ast-grep-ignore: no-long-names
class GuardrailUIAddGuardrailSettings(ResponseObject):
    supported_entities: list[str]
    supported_actions: list[str]
    supported_modes: list[str]
    supported_modes_by_provider: dict[str, list[str]]
    pii_entity_categories: list[PiiEntityCategoryMap]
    content_filter_settings: dict[str, JsonValue] | None = None
    decision_model_providers: tuple[str, ...] = Field(default_factory=tuple)


class PresidioPerRequestConfig(ResponseObject):
    language: str | None = None
    entities: list[PiiEntityType] | None = None


class ApplyGuardrailRequest(ResponseObject):
    guardrail_name: str
    text: str
    language: str | None = None
    entities: list[PiiEntityType] | None = None
    input_type: str = "request"
    messages: list[dict[str, JsonValue]] | None = None
    metadata: dict[str, JsonValue] | None = None


class ApplyGuardrailResponse(ResponseObject):
    response_text: str


class PatchGuardrailRequest(ResponseObject):
    guardrail_name: str | None = None
    litellm_params: BaseLitellmParams | None = None
    guardrail_info: dict[str, JsonValue] | None = None
