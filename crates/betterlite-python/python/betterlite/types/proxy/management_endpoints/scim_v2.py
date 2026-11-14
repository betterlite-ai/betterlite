from typing import ClassVar, Final, Unpack

from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    JsonValue,
    TypeAdapter,
    field_validator,
    model_validator,
)
from typing_extensions import override

from betterlite.types.utils.base import DumpOptions

SCIM_ENTERPRISE_USER_SCHEMA: Final[str] = (
    "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"
)
SCIM_ENTERPRISE_METADATA_KEY: Final[str] = "scim_enterprise"
SCIM_ENTITLEMENTS_METADATA_KEY: Final[str] = "scim_entitlements"
SCIM_ROLES_METADATA_KEY: Final[str] = "scim_roles"

SCIM_MANAGED_TEAM_METADATA_KEY: Final[str] = "scim_managed"
SCIM_TEAM_DATA_METADATA_KEY: Final[str] = "scim_data"

_ALIASED_CONFIG: Final[ConfigDict] = ConfigDict(
    serialize_by_alias=True, validate_by_name=True
)


def _absent(value: JsonValue | BaseModel) -> bool:
    match value:
        case None:
            return True
        case BaseModel() | str() | bool() | int() | float() | list() | dict():
            return False


class _SCIMBase(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(serialize_by_alias=True)


class UserScimMetadata(_SCIMBase):  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/proxy/management_endpoints/scim_v2.py#L25
    given_name: str | None = Field(default=None, alias="givenName")
    family_name: str | None = Field(default=None, alias="familyName")


LiteLLM_UserScimMetadata = UserScimMetadata


class SCIMResource(_SCIMBase):
    schemas: list[str]
    id: str | None = None
    external_id: str | None = Field(default=None, alias="externalId")
    meta: dict[str, JsonValue] | None = None


class SCIMUserName(_SCIMBase):
    family_name: str | None = Field(default=None, alias="familyName")
    given_name: str | None = Field(default=None, alias="givenName")
    formatted: str | None = None
    middle_name: str | None = Field(default=None, alias="middleName")
    honorific_prefix: str | None = Field(default=None, alias="honorificPrefix")
    honorific_suffix: str | None = Field(default=None, alias="honorificSuffix")


class SCIMUserEmail(_SCIMBase):
    value: str
    type: str | None = None
    primary: bool | None = None

    @field_validator("value")
    @classmethod
    def normalize_address(cls, value: str) -> str:
        stripped = value.strip()
        local, separator, domain = stripped.rpartition("@")
        return local + separator + domain.lower() if separator else stripped


class SCIMUserGroup(_SCIMBase):
    value: str
    display: str | None = None
    type: str | None = "direct"


class SCIMMultiValuedAttribute(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")

    value: str | None = None
    display: str | None = None
    type: str | None = None
    primary: bool | None = None

    @model_validator(mode="before")
    @classmethod
    def coerce_bare_string(cls, data: JsonValue | BaseModel) -> JsonValue | BaseModel:
        match data:
            case str():
                return {"value": data}
            case None | bool() | int() | float() | list() | dict() | BaseModel():
                return data


SCIM_MULTI_VALUED_LIST_ADAPTER: Final[TypeAdapter[list[SCIMMultiValuedAttribute]]] = TypeAdapter(
    list[SCIMMultiValuedAttribute]
)

# ast-grep-ignore: no-long-names
SCIM_MULTI_VALUED_ATTRIBUTE_METADATA_KEYS: Final[dict[str, str]] = {
    "entitlements": SCIM_ENTITLEMENTS_METADATA_KEY,
    "roles": SCIM_ROLES_METADATA_KEY,
}


class SCIMUserManager(_SCIMBase):
    model_config: ClassVar[ConfigDict] = _ALIASED_CONFIG

    value: str | None = None
    display_name: str | None = Field(default=None, alias="displayName")
    ref: str | None = Field(default=None, alias="$ref")


class SCIMEnterpriseUser(_SCIMBase):
    model_config: ClassVar[ConfigDict] = _ALIASED_CONFIG

    employee_number: str | None = Field(default=None, alias="employeeNumber")
    cost_center: str | None = Field(default=None, alias="costCenter")
    organization: str | None = None
    division: str | None = None
    department: str | None = None
    manager: SCIMUserManager | None = None


class SCIMUser(SCIMResource):
    model_config: ClassVar[ConfigDict] = _ALIASED_CONFIG

    user_name: str | None = Field(default=None, alias="userName")
    name: SCIMUserName | None = None
    display_name: str | None = Field(default=None, alias="displayName")
    active: bool = True
    emails: list[SCIMUserEmail] | None = None
    groups: list[SCIMUserGroup] | None = None
    entitlements: list[SCIMMultiValuedAttribute] | None = Field(
        default=None, exclude_if=_absent
    )
    roles: list[SCIMMultiValuedAttribute] | None = Field(default=None, exclude_if=_absent)
    enterprise_user: SCIMEnterpriseUser | None = Field(
        default=None,
        alias=SCIM_ENTERPRISE_USER_SCHEMA,
        serialization_alias=SCIM_ENTERPRISE_USER_SCHEMA,
        exclude_if=_absent,
    )


class SCIMMember(_SCIMBase):
    value: str
    display: str | None = None
    type: str | None = None

    @field_validator("type", mode="before")
    @classmethod
    def normalize_type(cls, value: JsonValue | BaseModel) -> str | None:
        match value:
            case str():
                return value
            case None | bool() | int() | float() | list() | dict() | BaseModel():
                return None


class SCIMGroup(SCIMResource):
    display_name: str = Field(alias="displayName")
    members: list[SCIMMember] | None = None


class SCIMPlaceholderMergeResult(_SCIMBase):
    placeholder_user_id: str
    merged_into_user_id: str
    team_ids: tuple[str, ...]


class SCIMListResponse(_SCIMBase):
    schemas: list[str] = ["urn:ietf:params:scim:api:messages:2.0:ListResponse"]
    total_results: int = Field(alias="totalResults")
    start_index: int | None = Field(default=1, alias="startIndex")
    items_per_page: int | None = Field(default=10, alias="itemsPerPage")
    resources: list[SCIMUser] | list[SCIMGroup] = Field(alias="Resources")


class SCIMPatchOperation(_SCIMBase):
    op: str
    path: str | None = None
    value: JsonValue = None

    @field_validator("op")
    @classmethod
    def normalize_op(cls, value: str) -> str:
        normalized = value.lower()
        if normalized not in {"add", "remove", "replace"}:
            raise ValueError("op must be add, remove, or replace")
        return normalized


class SCIMPatchOp(_SCIMBase):
    schemas: list[str] = ["urn:ietf:params:scim:api:messages:2.0:PatchOp"]
    operations: list[SCIMPatchOperation] = Field(alias="Operations")


class SCIMFeature(_SCIMBase):
    supported: bool
    max_operations: int | None = Field(default=None, alias="maxOperations")
    max_payload_size: int | None = Field(default=None, alias="maxPayloadSize")
    max_results: int | None = Field(default=None, alias="maxResults")


class SCIMServiceProviderConfig(_SCIMBase):
    schemas: list[str] = ["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig"]
    patch: SCIMFeature = SCIMFeature(supported=True)
    bulk: SCIMFeature = SCIMFeature(supported=False)
    filter: SCIMFeature = SCIMFeature(supported=False)
    change_password: SCIMFeature = Field(
        default=SCIMFeature(supported=False), alias="changePassword"
    )
    sort: SCIMFeature = SCIMFeature(supported=False)
    etag: SCIMFeature = SCIMFeature(supported=False)
    authentication_schemes: list[dict[str, JsonValue]] | None = Field(
        default=None, alias="authenticationSchemes"
    )
    meta: dict[str, JsonValue] | None = None


class SCIMSchemaExtension(_SCIMBase):
    model_config: ClassVar[ConfigDict] = _ALIASED_CONFIG

    schema_: str
    required: bool

    @override
    def model_dump(self, **kwargs: Unpack[DumpOptions]) -> dict[str, JsonValue]:
        dumped: dict[str, JsonValue] = super().model_dump(**kwargs)
        dumped["schema"] = dumped.pop("schema_")
        return dumped


class SCIMResourceType(_SCIMBase):
    model_config: ClassVar[ConfigDict] = _ALIASED_CONFIG

    schemas: list[str] = ["urn:ietf:params:scim:schemas:core:2.0:ResourceType"]
    id: str
    name: str
    description: str | None = None
    endpoint: str
    schema_: str
    schema_extensions: list[SCIMSchemaExtension] | None = Field(
        default=None, alias="schemaExtensions"
    )
    meta: dict[str, JsonValue] | None = None

    @override
    def model_dump(self, **kwargs: Unpack[DumpOptions]) -> dict[str, JsonValue]:
        dumped: dict[str, JsonValue] = super().model_dump(**kwargs)
        dumped["schema"] = dumped.pop("schema_")
        if dumped.get("schemaExtensions") is None:
            dumped.pop("schemaExtensions", None)
        if dumped.get("schema_extensions") is None:
            dumped.pop("schema_extensions", None)
        return dumped


class SCIMSchemaAttribute(_SCIMBase):
    name: str
    type: str
    multi_valued: bool = Field(default=False, alias="multiValued")
    description: str | None = None
    required: bool = False
    mutability: str = "readWrite"
    returned: str = "default"
    uniqueness: str = "none"
    sub_attributes: list["SCIMSchemaAttribute"] | None = Field(
        default=None, alias="subAttributes"
    )

    @override
    def model_dump(self, **kwargs: Unpack[DumpOptions]) -> dict[str, JsonValue]:
        dumped: dict[str, JsonValue] = super().model_dump(**kwargs)
        if dumped.get("subAttributes") is None:
            dumped.pop("subAttributes", None)
        if dumped.get("sub_attributes") is None:
            dumped.pop("sub_attributes", None)
        return dumped


class SCIMSchema(_SCIMBase):
    schemas: list[str] = ["urn:ietf:params:scim:schemas:core:2.0:Schema"]
    id: str
    name: str
    description: str | None = None
    attributes: list[SCIMSchemaAttribute] = []
    meta: dict[str, JsonValue] | None = None
