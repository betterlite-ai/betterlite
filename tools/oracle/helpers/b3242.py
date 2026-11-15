import asyncio
from datetime import datetime
from typing import Literal, TypeAlias, TypedDict

from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.policy_engine.policy_registry import PolicyRegistry
from litellm.proxy.utils import PrismaClient
from litellm.types.proxy.policy_engine import resolver_types
from litellm.types.proxy.policy_engine.policy_types import (
    Policy,
    PolicyGuardrails,
    PolicyScope,
)
from litellm.types.proxy.policy_engine.resolver_types import (
    AttachmentImpactResponse,
    PipelineTestRequest,
    PolicyAttachmentCreateRequest,
    PolicyAttachmentDBResponse,
    PolicyAttachmentListResponse,
    PolicyConditionRequest,
    PolicyCreateRequest,
    PolicyDBResponse,
    PolicyGuardrailsResponse,
    PolicyInfoResponse,
    PolicyListDBResponse,
    PolicyListResponse,
    PolicyMatchDetail,
    PolicyResolveRequest,
    PolicyResolveResponse,
    PolicyScopeResponse,
    PolicySummaryItem,
    PolicyTestResponse,
    PolicyUpdateRequest,
    PolicyVersionCompareResponse,
    PolicyVersionCreateRequest,
    PolicyVersionListResponse,
    ResolvedPolicy,
)
from pydantic import BaseModel, JsonValue, TypeAdapter

DUMP = TypeAdapter(dict[str, JsonValue])

RECORDS: dict[str, type[BaseModel]] = {
    "PolicyScopeResponse": PolicyScopeResponse,
    "PolicyGuardrailsResponse": PolicyGuardrailsResponse,
    "PolicyInfoResponse": PolicyInfoResponse,
    "PolicySummaryItem": PolicySummaryItem,
    "PolicyListResponse": PolicyListResponse,
    "PolicyTestResponse": PolicyTestResponse,
    "PolicyConditionRequest": PolicyConditionRequest,
    "PolicyCreateRequest": PolicyCreateRequest,
    "PolicyUpdateRequest": PolicyUpdateRequest,
    "PolicyDBResponse": PolicyDBResponse,
    "PolicyListDBResponse": PolicyListDBResponse,
    "PolicyVersionCreateRequest": PolicyVersionCreateRequest,
    "PolicyVersionStatusUpdateRequest": resolver_types.PolicyVersionStatusUpdateRequest,
    "PolicyVersionListResponse": PolicyVersionListResponse,
    "PolicyVersionCompareResponse": PolicyVersionCompareResponse,
    "PolicyAttachmentCreateRequest": PolicyAttachmentCreateRequest,
    "PolicyAttachmentDBResponse": PolicyAttachmentDBResponse,
    "PolicyAttachmentListResponse": PolicyAttachmentListResponse,
    "PipelineTestRequest": PipelineTestRequest,
    "PolicyResolveRequest": PolicyResolveRequest,
    "PolicyResolveResponse": PolicyResolveResponse,
    "PolicyMatchDetail": PolicyMatchDetail,
    "AttachmentImpactResponse": AttachmentImpactResponse,
}


class ValidateCall(TypedDict):
    group: Literal["policy_records_records"]
    operation: Literal["validate"]
    model: str
    value: dict[str, JsonValue]


class ProjectScopeCall(TypedDict):
    group: Literal["policy_records_records"]
    operation: Literal["project_scope"]
    scope: dict[str, JsonValue]


class ProjectGuardrailsCall(TypedDict):
    group: Literal["policy_records_records"]
    operation: Literal["project_guardrails"]
    guardrails: dict[str, JsonValue]


class ProjectInfoCall(TypedDict):
    group: Literal["policy_records_records"]
    operation: Literal["project_info"]
    policy_name: str
    policy: dict[str, JsonValue]
    resolved: dict[str, JsonValue]


class LookupCall(TypedDict):
    group: Literal["policy_records_lookup"]
    operation: Literal["lookup"]
    policy_id: str
    row: dict[str, JsonValue] | None
    failure: str | None


Input: TypeAlias = (
    ValidateCall | ProjectScopeCall | ProjectGuardrailsCall | ProjectInfoCall | LookupCall
)


class FakePolicyRow(BaseModel):
    policy_id: str
    policy_name: str
    version_number: int
    version_status: str
    parent_version_id: str | None = None
    is_latest: bool = True
    published_at: datetime | None = None
    production_at: datetime | None = None
    inherit: str | None = None
    description: str | None = None
    guardrails_add: list[str] | None = None
    guardrails_remove: list[str] | None = None
    condition: dict[str, JsonValue] | None = None
    pipeline: dict[str, JsonValue] | None = None
    created_at: datetime
    updated_at: datetime
    created_by: str | None = None
    updated_by: str | None = None


class FakePolicyTable:
    def __init__(self, row: FakePolicyRow | None, failure: str | None) -> None:
        self.row = row
        self.failure = failure

    async def find_unique(self, where: dict[str, str]) -> FakePolicyRow | None:
        del where
        await asyncio.sleep(0)
        if self.failure is not None:
            raise RuntimeError(self.failure)
        return self.row


class FakeDb(PrismaWrapper):
    def __init__(self, table: FakePolicyTable) -> None:
        self.litellm_policytable = table


class FakePrismaClient(PrismaClient):
    def __init__(self, table: FakePolicyTable) -> None:
        self.db = FakeDb(table)


def validate(input_value: ValidateCall) -> JsonValue:
    model = RECORDS[input_value["model"]]
    return DUMP.validate_json(model.model_validate(input_value["value"]).model_dump_json())


def project_scope(input_value: ProjectScopeCall) -> JsonValue:
    scope = PolicyScope.model_validate(input_value["scope"])
    response = PolicyScopeResponse(
        teams=scope.get_teams(),
        keys=scope.get_keys(),
        models=scope.get_models(),
        tags=scope.get_tags(),
    )
    return DUMP.validate_json(response.model_dump_json())


def project_guardrails(input_value: ProjectGuardrailsCall) -> JsonValue:
    guardrails = PolicyGuardrails.model_validate(input_value["guardrails"])
    response = PolicyGuardrailsResponse(
        add=guardrails.get_add(),
        remove=guardrails.get_remove(),
    )
    return DUMP.validate_json(response.model_dump_json())


def project_info(input_value: ProjectInfoCall) -> JsonValue:
    policy = Policy.model_validate(input_value["policy"])
    resolved = ResolvedPolicy.model_validate(input_value["resolved"])
    response = PolicyInfoResponse(
        policy_name=input_value["policy_name"],
        inherit=policy.inherit,
        scope=PolicyScopeResponse(),
        guardrails=PolicyGuardrailsResponse(
            add=policy.guardrails.get_add(),
            remove=policy.guardrails.get_remove(),
        ),
        resolved_guardrails=resolved.guardrails,
        inheritance_chain=resolved.inheritance_chain,
    )
    return DUMP.validate_json(response.model_dump_json())


def lookup(input_value: LookupCall) -> JsonValue:
    row_value = input_value["row"]
    row = None if row_value is None else FakePolicyRow.model_validate(row_value)
    client = FakePrismaClient(FakePolicyTable(row, input_value["failure"]))
    response = asyncio.run(
        PolicyRegistry().get_policy_by_id_from_db(input_value["policy_id"], client)
    )
    if response is None:
        return None
    return DUMP.validate_json(response.model_dump_json())


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "validate":
        return validate(input_value)
    if input_value["operation"] == "project_scope":
        return project_scope(input_value)
    if input_value["operation"] == "project_guardrails":
        return project_guardrails(input_value)
    if input_value["operation"] == "project_info":
        return project_info(input_value)
    return lookup(input_value)
