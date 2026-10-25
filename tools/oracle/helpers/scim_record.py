from typing import Any

from pydantic import BaseModel, ConfigDict, Field, field_validator

ENTERPRISE = "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"
PATCH = "urn:ietf:params:scim:api:messages:2.0:PatchOp"


class MultiValue(BaseModel):
    model_config = ConfigDict(extra="allow")
    value: str | None = None
    display: str | None = None
    type: str | None = None
    primary: bool | None = None

    @classmethod
    def from_value(cls, value: Any) -> "MultiValue":
        return cls.model_validate({"value": value} if isinstance(value, str) else value)


class Manager(BaseModel):
    model_config = ConfigDict(populate_by_name=True)
    value: str | None = None
    displayName: str | None = None
    ref: str | None = Field(default=None, alias="$ref")


class Enterprise(BaseModel):
    model_config = ConfigDict(populate_by_name=True)
    employeeNumber: str | None = None
    costCenter: str | None = None
    organization: str | None = None
    division: str | None = None
    department: str | None = None
    manager: Manager | None = None


class Name(BaseModel):
    familyName: str | None = None
    givenName: str | None = None
    formatted: str | None = None
    middleName: str | None = None
    honorificPrefix: str | None = None
    honorificSuffix: str | None = None


class Email(BaseModel):
    value: str
    type: str | None = None
    primary: bool | None = None


class GroupReference(BaseModel):
    value: str
    display: str | None = None
    type: str | None = "direct"


class User(BaseModel):
    model_config = ConfigDict(populate_by_name=True)
    schemas: list[str]
    id: str | None = None
    externalId: str | None = None
    meta: dict[str, Any] | None = None
    userName: str | None = None
    name: Name | None = None
    displayName: str | None = None
    active: bool = True
    emails: list[Email] | None = None
    groups: list[GroupReference] | None = None
    entitlements: list[MultiValue] | None = None
    roles: list[MultiValue] | None = None
    enterprise: Enterprise | None = Field(default=None, alias=ENTERPRISE)

    @classmethod
    def model_validate(cls, obj: Any, **kwargs: Any) -> "User":
        if isinstance(obj, dict):
            obj = dict(obj)
            for key in ("entitlements", "roles"):
                if key in obj and isinstance(obj[key], list):
                    obj[key] = [MultiValue.from_value(value) for value in obj[key]]
        return super().model_validate(obj, **kwargs)

    def model_dump(self, **kwargs: Any) -> dict[str, Any]:
        result = super().model_dump(by_alias=True, **kwargs)
        if self.enterprise is None:
            result.pop(ENTERPRISE, None)
        for key in ("entitlements", "roles"):
            if getattr(self, key) is None:
                result.pop(key, None)
        return result


class Member(BaseModel):
    value: str
    display: str | None = None
    type: str | None = None

    @field_validator("type", mode="before")
    @classmethod
    def discard_non_string(cls, value: Any) -> str | None:
        return value if isinstance(value, str) else None


class Group(BaseModel):
    schemas: list[str]
    id: str | None = None
    externalId: str | None = None
    meta: dict[str, Any] | None = None
    displayName: str
    members: list[Member] | None = None


class Operation(BaseModel):
    op: str
    path: str | None = None
    value: Any = None

    @field_validator("op", mode="before")
    @classmethod
    def normalize(cls, value: Any) -> str:
        lowered = value.lower()
        if lowered not in {"add", "remove", "replace"}:
            raise ValueError("op must be add, remove, or replace")
        return lowered


class PatchOp(BaseModel):
    schemas: list[str] = [PATCH]
    Operations: list[Operation]


def run(input_value: dict[str, Any]) -> Any:
    kind = input_value["kind"]
    value = input_value["value"]
    if kind == "user":
        return User.model_validate(value).model_dump()
    if kind == "group":
        return Group.model_validate(value).model_dump()
    if kind == "patch":
        return PatchOp.model_validate(value).model_dump()
    raise ValueError(f"unknown SCIM resource kind: {kind}")
