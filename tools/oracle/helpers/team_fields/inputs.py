from typing import Final, Literal, TypeAlias, TypedDict

from pydantic import JsonValue, TypeAdapter

ValidatorKind: TypeAlias = Literal[
    "absent",
    "function",
    "accept",
    "sync",
    "reject",
    "reject_empty",
    "raises",
    "malformed_type",
    "malformed_object",
    "sleep",
]
RoleName: TypeAlias = Literal["proxy_admin", "org_admin", "team_admin"]


class RequesterDict(TypedDict):
    user_id: str | None
    user_email: str | None
    user_role: RoleName | None


class PayloadDict(TypedDict):
    operation: Literal["create", "update"]
    metadata: dict[str, JsonValue]
    existing_metadata: dict[str, JsonValue] | None
    team_id: str | None
    team_alias: str | None
    requester: RequesterDict


PAYLOAD: Final[TypeAdapter[PayloadDict]] = TypeAdapter(PayloadDict)


class FieldDict(TypedDict):
    key: str
    label: str | None


class SchemaInput(TypedDict):
    function: Literal["schema"]
    schema: JsonValue


class SchemaAnswer(TypedDict, total=False):
    fields: list[FieldDict]
    failed: bool


class RegistryInput(TypedDict):
    function: Literal["registry"]
    steps: list[Literal["get", "set_a", "set_b", "clear"]]


class RegistryAnswer(TypedDict):
    seen: list[Literal["none", "a", "b"]]


class ValidateInput(TypedDict):
    function: Literal["validate"]
    general_settings: dict[str, JsonValue]
    validator: ValidatorKind
    operation: Literal["create", "update"]
    metadata: JsonValue
    existing_metadata: JsonValue
    team_id: str | None
    team_alias: str | None
    requester: RequesterDict


class ValidateAnswer(TypedDict):
    status: int | None
    error: str | None
    payload: PayloadDict | None


class SanitizeInput(TypedDict):
    function: Literal["sanitize"]
    value: str


class SanitizeAnswer(TypedDict):
    text: str


class EditAccessInput(TypedDict):
    function: Literal["edit_access"]
    role: RoleName | None
    general_settings: dict[str, JsonValue]


class EditAccessAnswer(TypedDict):
    kind: Literal["unrestricted", "team_admin", "team_admin_disabled", "none"]
    editable_fields: list[str] | None


Input: TypeAlias = SchemaInput | RegistryInput | ValidateInput | SanitizeInput | EditAccessInput
Answer: TypeAlias = (
    SchemaAnswer | RegistryAnswer | ValidateAnswer | SanitizeAnswer | EditAccessAnswer
)
