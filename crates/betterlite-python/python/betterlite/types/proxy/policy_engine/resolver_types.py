from datetime import datetime
from typing import ClassVar, Literal

from pydantic import ConfigDict, Field, JsonValue

from betterlite.types.utils.base import ResponseObject


class PolicyMatchContext(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    team_alias: str | None = None
    key_alias: str | None = None
    model: str | None = None
    tags: list[str] | None = None


class ResolvedPolicy(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    policy_name: str
    guardrails: list[str] = Field(default_factory=list)
    inheritance_chain: list[str] = Field(default_factory=list)


class PolicyScopeResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    teams: list[str] = Field(default_factory=list)
    keys: list[str] = Field(default_factory=list)
    models: list[str] = Field(default_factory=list)
    tags: list[str] = Field(default_factory=list)


class PolicyGuardrailsResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    add: list[str] = Field(default_factory=list)
    remove: list[str] = Field(default_factory=list)


class PolicyInfoResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    policy_name: str
    inherit: str | None = None
    scope: PolicyScopeResponse
    guardrails: PolicyGuardrailsResponse
    resolved_guardrails: list[str]
    inheritance_chain: list[str]


class PolicySummaryItem(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    inherit: str | None = None
    scope: PolicyScopeResponse
    guardrails: PolicyGuardrailsResponse
    resolved_guardrails: list[str]
    inheritance_chain: list[str]


class PolicyListResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    policies: dict[str, PolicySummaryItem]
    total_count: int


class PolicyTestResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    context: PolicyMatchContext
    matching_policies: list[str]
    resolved_guardrails: list[str]
    message: str | None = None


class PolicyConditionRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    model: str | None = None


class PolicyCreateRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    policy_name: str
    inherit: str | None = None
    description: str | None = None
    guardrails_add: list[str] | None = None
    guardrails_remove: list[str] | None = None
    condition: PolicyConditionRequest | None = None
    pipeline: dict[str, JsonValue] | None = None


class PolicyUpdateRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    policy_name: str | None = None
    inherit: str | None = None
    description: str | None = None
    guardrails_add: list[str] | None = None
    guardrails_remove: list[str] | None = None
    condition: PolicyConditionRequest | None = None
    pipeline: dict[str, JsonValue] | None = None


class PolicyDBResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    policy_id: str
    policy_name: str
    version_number: int = 1
    version_status: str = "production"
    parent_version_id: str | None = None
    is_latest: bool = True
    published_at: datetime | None = None
    production_at: datetime | None = None
    inherit: str | None = None
    description: str | None = None
    guardrails_add: list[str] = Field(default_factory=list)
    guardrails_remove: list[str] = Field(default_factory=list)
    condition: dict[str, JsonValue] | None = None
    pipeline: dict[str, JsonValue] | None = None
    created_at: datetime | None = None
    updated_at: datetime | None = None
    created_by: str | None = None
    updated_by: str | None = None
    definition_location: Literal["db", "config"] = "db"


class PolicyListDBResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    policies: list[PolicyDBResponse] = Field(default_factory=list)
    total_count: int = 0


class PolicyVersionCreateRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    source_policy_id: str | None = None


# ast-grep-ignore: no-long-names
class PolicyVersionStatusUpdateRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    version_status: str


class PolicyVersionListResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    policy_name: str
    versions: list[PolicyDBResponse] = Field(default_factory=list)
    total_count: int = 0


class PolicyVersionCompareResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    version_a: PolicyDBResponse
    version_b: PolicyDBResponse
    field_diffs: dict[str, dict[str, JsonValue]] = Field(default_factory=dict)


class PolicyAttachmentCreateRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    policy_name: str
    scope: str | None = None
    teams: list[str] | None = None
    keys: list[str] | None = None
    models: list[str] | None = None
    tags: list[str] | None = None
    priority: int | None = Field(default=None, ge=-2147483648, le=2147483647)
    default: bool = False


class PolicyAttachmentDBResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    attachment_id: str
    policy_name: str
    scope: str | None = None
    teams: list[str] = Field(default_factory=list)
    keys: list[str] = Field(default_factory=list)
    models: list[str] = Field(default_factory=list)
    tags: list[str] = Field(default_factory=list)
    priority: int | None = None
    default: bool = False
    created_at: datetime | None = None
    updated_at: datetime | None = None
    created_by: str | None = None
    updated_by: str | None = None
    definition_location: Literal["db", "config"] = "db"


class PolicyAttachmentListResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    attachments: list[PolicyAttachmentDBResponse] = Field(default_factory=list)
    total_count: int = 0


class PipelineTestRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    pipeline: dict[str, JsonValue]
    test_messages: list[dict[str, str]]


class PolicyResolveRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    team_alias: str | None = None
    key_alias: str | None = None
    model: str | None = None
    tags: list[str] | None = None


class PolicyMatchDetail(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    policy_name: str
    matched_via: str
    guardrails_added: list[str] = Field(default_factory=list)


class PolicyResolveResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    effective_guardrails: list[str] = Field(default_factory=list)
    matched_policies: list[PolicyMatchDetail] = Field(default_factory=list)


class AttachmentImpactResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    affected_keys_count: int = 0
    affected_teams_count: int = 0
    unnamed_keys_count: int = 0
    unnamed_teams_count: int = 0
    sample_keys: list[str] = Field(default_factory=list)
    sample_teams: list[str] = Field(default_factory=list)
