# ruff: noqa: N801 - LiteLLM's class names, kept for parity
from typing import Literal, TypedDict

from pydantic import Field

from betterlite.types.llms.base import LiteLLMPydanticObjectBase


# ast-grep-ignore: no-long-names
class LiteLLM_UpperboundKeyGenerateParams(LiteLLMPydanticObjectBase):  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/proxy/management_endpoints/ui_sso.py#L10
    max_budget: float | None = None
    budget_duration: str | None = None
    duration: str | None = None
    max_parallel_requests: int | None = None
    tpm_limit: int | None = None
    rpm_limit: int | None = None


# ast-grep-ignore: no-long-names
class MicrosoftGraphAPIUserGroupDirectoryObject(TypedDict, total=False):
    odata_type: str | None
    id: str | None
    deletedDateTime: str | None
    description: str | None
    displayName: str | None
    roleTemplateId: str | None


# ast-grep-ignore: no-long-names
class MicrosoftGraphAPIUserGroupResponse(TypedDict, total=False):
    odata_context: str | None
    odata_nextLink: str | None
    # ast-grep-ignore: no-long-names
    value: list[MicrosoftGraphAPIUserGroupDirectoryObject] | None


# ast-grep-ignore: no-long-names
class MicrosoftServicePrincipalTeam(TypedDict, total=False):
    principalDisplayName: str | None
    principalId: str | None


class AccessControl_UI_AccessMode(LiteLLMPydanticObjectBase):  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/proxy/management_endpoints/ui_sso.py#L57
    type: Literal["restricted_sso_group"]
    restricted_sso_group: str
    sso_group_jwt_field: str


class RoleMappings(LiteLLMPydanticObjectBase):
    provider: str
    group_claim: str
    default_role: str | None = None
    roles: dict[str, list[str]] = Field(default_factory=dict)


class TeamMappings(LiteLLMPydanticObjectBase):
    team_ids_jwt_field: str | None = None


class SSOConfig(LiteLLMPydanticObjectBase):
    google_client_id: str | None = None
    google_client_secret: str | None = None
    microsoft_client_id: str | None = None
    microsoft_client_secret: str | None = None
    microsoft_tenant: str | None = None
    generic_client_id: str | None = None
    generic_client_secret: str | None = None
    generic_authorization_endpoint: str | None = None
    generic_token_endpoint: str | None = None
    generic_userinfo_endpoint: str | None = None
    generic_scope: str | None = None
    saml_idp_metadata_url: str | None = None
    saml_idp_metadata_xml: str | None = None
    saml_sp_entity_id: str | None = None
    saml_allow_unsolicited: str | None = None
    proxy_base_url: str | None = None
    user_email: str | None = None
    ui_access_mode: AccessControl_UI_AccessMode | str | None = None
    role_mappings: RoleMappings | None = None
    team_mappings: TeamMappings | None = None


class DefaultTeamSSOParams(LiteLLMPydanticObjectBase):
    models: list[str] = Field(default=[])
    max_budget: float | None = None
    budget_duration: str | None = None
    tpm_limit: int | None = None
    rpm_limit: int | None = None
    team_member_permissions: list[str] | None = None
    organization_id: str | None = None
