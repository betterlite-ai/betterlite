from datetime import datetime
from typing import ClassVar, Final, Literal, TypeAlias

from pydantic import ConfigDict, Field
from typing_extensions import override

from betterlite.types.utils import JsonValue, ResponseObject
from betterlite.types.utils.base import InitValue

_ModelGroupMode: TypeAlias = Literal[
    "chat",
    "embedding",
    "completion",
    "image_generation",
    "audio_transcription",
    "rerank",
    "moderations",
]

_BOOL_FIELDS: Final[tuple[str, ...]] = (
    "supports_parallel_function_calling",
    "supports_vision",
    "supports_web_search",
    "supports_url_context",
    "supports_reasoning",
    "supports_function_calling",
    "supports_fast_mode",
    "is_public_model_group",
)


class ModelGroupInfoProxy(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    model_group: str
    providers: list[str]
    max_input_tokens: float | None = None
    max_output_tokens: float | None = None
    input_cost_per_token: float | None = None
    output_cost_per_token: float | None = None
    input_cost_per_pixel: float | None = None
    mode: str | _ModelGroupMode | None = "chat"
    tpm: int | None = None
    rpm: int | None = None
    itpm: int | None = None
    otpm: int | None = None
    # ast-grep-ignore: no-long-names
    supports_parallel_function_calling: bool = False
    supports_vision: bool = False
    supports_web_search: bool = False
    supports_url_context: bool = False
    supports_reasoning: bool = False
    supports_function_calling: bool = False
    supports_fast_mode: bool = False
    supported_reasoning_efforts: tuple[str, ...] | None = None
    supported_openai_params: list[str] | None = Field(default_factory=list)
    # ast-grep-ignore: no-long-names
    configurable_clientside_auth_params: list[str | dict[str, JsonValue]] | None = None
    is_public_model_group: bool = False
    health_status: str | None = None
    health_response_time: float | None = None
    health_checked_at: str | None = None

    @override
    def __init__(self, **data: InitValue) -> None:
        for name in _BOOL_FIELDS:
            if data.get(name) is None:
                data[name] = False
        super().__init__(**data)


class UpdateUsefulLinksRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    useful_links: dict[str, str | dict[str, JsonValue]]


# ast-grep-ignore: no-long-names
class AutoRouterClassifierDefaultPromptResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    system_prompt: str


class NewModelGroupRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    access_group: str
    model_names: list[str] | None = None
    model_ids: list[str] | None = None


class NewModelGroupResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    access_group: str
    model_names: list[str] | None = None
    model_ids: list[str] | None = None
    models_updated: int


class UpdateModelGroupRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    model_names: list[str] | None = None
    model_ids: list[str] | None = None


# ast-grep-ignore: no-long-names
class DeleteAccessGroupBudgetResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    access_group: str
    budget_deleted: bool
    message: str


class AccessGroupBudget(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    budget_id: str
    max_budget: float | None = None
    soft_budget: float | None = None
    budget_duration: str | None = None
    budget_reset_at: datetime | None = None


class AccessGroupBudgetRequest(ResponseObject):
    model_config = ConfigDict(extra="forbid")

    budget_id: str | None = None
    max_budget: float | None = Field(default=None, ge=0)
    soft_budget: float | None = Field(default=None, ge=0)
    budget_duration: str | None = None


class AccessGroupBudgetResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    access_group: str
    spend: float
    budget: AccessGroupBudget | None = None


class DeleteModelGroupResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    access_group: str
    models_updated: int
    message: str


class AccessGroupInfo(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    access_group: str
    model_names: list[str]
    deployment_count: int
    spend: float | None = None
    budget: AccessGroupBudget | None = None


class ListAccessGroupsResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    access_groups: list[AccessGroupInfo]
