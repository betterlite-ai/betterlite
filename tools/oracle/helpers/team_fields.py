import asyncio
from collections.abc import Mapping
from typing import Final, Literal, TypeAlias, TypedDict, assert_never, cast

from litellm.proxy import proxy_server
from litellm.proxy._types import (
    LitellmUserRoles,
    TeamEditAsTeamAdmin,
    TeamEditAsTeamAdminDisabled,
    TeamEditNone,
    TeamEditUnrestricted,
    UserAPIKeyAuth,
)
from litellm.proxy.management_endpoints import team_endpoints
from litellm.proxy.management_helpers.team_metadata_validation import (
    TeamMetadataValidationPayload,
    TeamMetadataValidationResult,
    TeamMetadataValidator,
    TeamMetadataValidatorRegistry,
    parse_team_metadata_schema,
    validate_team_metadata_if_configured,
)
from pydantic import BaseModel, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException as StarletteHTTPException

ValidatorKind: TypeAlias = Literal[
    "absent", "function", "accept", "sync", "reject", "reject_empty",
    "raises", "malformed_type", "malformed_object", "sleep",
]
RoleName: TypeAlias = Literal["proxy_admin", "org_admin", "team_admin"]


class RequesterDict(TypedDict):
    user_id: str | None
    user_email: str | None
    user_role: Literal[
        "proxy_admin", "proxy_admin_viewer", "org_admin", "internal_user",
        "internal_user_viewer", "team", "customer",
    ] | None


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


class RegistryInput(TypedDict):
    function: Literal["registry"]
    steps: list[Literal["get", "set_a", "set_b", "clear"]]


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


class SanitizeInput(TypedDict):
    function: Literal["sanitize"]
    value: str


class EditAccessInput(TypedDict):
    function: Literal["edit_access"]
    role: RoleName | None
    general_settings: dict[str, JsonValue]


class Answer(TypedDict, total=False):
    fields: list[FieldDict]
    failed: bool
    seen: list[Literal["none", "a", "b"]]
    status: int | None
    error: str | None
    payload: PayloadDict | None
    text: str
    kind: Literal["unrestricted", "team_admin", "team_admin_disabled", "none"]
    editable_fields: list[str] | None


Input: TypeAlias = SchemaInput | RegistryInput | ValidateInput | SanitizeInput | EditAccessInput


class ErrorEnvelope(BaseModel):
    status_code: int
    detail: dict[str, str]


ERROR_ENVELOPE: Final[TypeAdapter[ErrorEnvelope]] = TypeAdapter(ErrorEnvelope)


class Recording:
    def __init__(
        self,
        result: TeamMetadataValidationResult | JsonValue,
        *,
        error: Exception | None = None,
        sleep: float = 0.0,
    ) -> None:
        self.payloads: list[PayloadDict] = []
        self._result = result
        self._error = error
        self._sleep = sleep

    async def __call__(
        self, payload: TeamMetadataValidationPayload
    ) -> TeamMetadataValidationResult | JsonValue:
        self.payloads.append(PAYLOAD.validate_json(payload.model_dump_json()))
        if self._error is not None:
            raise self._error
        if self._sleep:
            await asyncio.sleep(self._sleep)
        return self._result


class SyncRecording:
    def __init__(self) -> None:
        self.payloads: list[PayloadDict] = []

    def __call__(self, _payload: TeamMetadataValidationPayload) -> TeamMetadataValidationResult:
        return TeamMetadataValidationResult(valid=True)


class Marker:
    async def __call__(
        self, _payload: TeamMetadataValidationPayload
    ) -> TeamMetadataValidationResult:
        return TeamMetadataValidationResult(valid=True)


VALIDATOR_A = Marker()
VALIDATOR_B = Marker()


async def _accept_function(  # ast-grep-ignore: no-one-line-functions
    _payload: TeamMetadataValidationPayload,
) -> TeamMetadataValidationResult:
    return TeamMetadataValidationResult(valid=True)


VALIDATION_RESULTS: Final[dict[ValidatorKind, TeamMetadataValidationResult]] = {
    "accept": TeamMetadataValidationResult(valid=True),
    "reject": TeamMetadataValidationResult(valid=False, error_message="alias must be set"),
    "reject_empty": TeamMetadataValidationResult(valid=False, error_message=""),
    "raises": TeamMetadataValidationResult(valid=True),
    "sleep": TeamMetadataValidationResult(valid=True),
}


def build_validator(
    kind: ValidatorKind,
) -> tuple[TeamMetadataValidator | None, list[PayloadDict]]:
    if kind == "absent":
        return None, []
    if kind == "function":
        return _accept_function, []
    if kind == "sync":
        sync = SyncRecording()
        return cast("TeamMetadataValidator", sync), sync.payloads  # cast-ok: the async check rejects a plain callable before it runs  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/team_metadata_validation.py#L118-L124
    if kind in {"malformed_type", "malformed_object"}:
        raw: JsonValue = {"valid": "yes"} if kind == "malformed_type" else "ok"
        recorder = Recording(raw)
    else:
        recorder = Recording(
            VALIDATION_RESULTS[kind],
            error=ValueError("database exploded secret-token-123") if kind == "raises" else None,
            sleep=0.2 if kind == "sleep" else 0.0,
        )
    return cast("TeamMetadataValidator", recorder), recorder.payloads  # cast-ok: a malformed result is exactly what the guarded model_validate must reject  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/team_metadata_validation.py#L126-L133


def validate_answer(input_value: ValidateInput) -> Answer:
    proxy_server.general_settings = dict(input_value["general_settings"])
    proxy_server.premium_user = True
    validator, payloads = build_validator(input_value["validator"])
    registry = TeamMetadataValidatorRegistry()
    registry.set(validator)
    requester = input_value["requester"]
    role = requester["user_role"]
    caller = UserAPIKeyAuth(
        user_id=requester["user_id"],
        user_email=requester["user_email"],
        user_role=LitellmUserRoles(role) if role is not None else None,
    )
    try:
        asyncio.run(
            validate_team_metadata_if_configured(
                operation=input_value["operation"],
                metadata=cast("Mapping[str, JsonValue] | None", input_value["metadata"]),  # cast-ok: the case sends a non-dict value so the function's isinstance guard runs  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/team_metadata_validation.py#L171-L182
                existing_metadata=cast("Mapping[str, JsonValue] | None", input_value["existing_metadata"]),  # cast-ok: the case sends a non-dict value so the function's isinstance guard runs  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/team_metadata_validation.py#L171-L182
                team_id=input_value["team_id"],
                team_alias=input_value["team_alias"],
                user_api_key_dict=caller,
                registry=registry,
            )
        )
    except StarletteHTTPException as error:
        rejected = ERROR_ENVELOPE.validate_python(error, from_attributes=True)
        last = payloads[-1] if payloads else None
        return {"status": rejected.status_code, "error": rejected.detail["error"], "payload": last}
    return {"status": None, "error": None, "payload": payloads[-1] if payloads else None}


def registry_answer(input_value: RegistryInput) -> Answer:
    registry = TeamMetadataValidatorRegistry()
    seen: list[Literal["none", "a", "b"]] = []
    for step in input_value["steps"]:
        match step:
            case "get":
                current = registry.get()
                if current is None:
                    seen.append("none")
                elif current is VALIDATOR_A:
                    seen.append("a")
                else:
                    seen.append("b")
            case "set_a":
                registry.set(VALIDATOR_A)
            case "set_b":
                registry.set(VALIDATOR_B)
            case "clear":
                registry.set(None)
    return {"seen": seen}


def edit_access_answer(input_value: EditAccessInput) -> Answer:
    access = team_endpoints._caller_edit_access(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L490-L505
        input_value["role"], input_value["general_settings"]
    )
    match access:
        case TeamEditAsTeamAdmin():
            return {"kind": "team_admin", "editable_fields": list(access.editable_fields)}
        case TeamEditUnrestricted():
            return {"kind": "unrestricted", "editable_fields": None}
        case TeamEditAsTeamAdminDisabled():
            return {"kind": "team_admin_disabled", "editable_fields": None}
        case TeamEditNone():
            return {"kind": "none", "editable_fields": None}
    assert_never(access)


def run(input_value: Input) -> Answer:
    match input_value["function"]:
        case "schema":
            try:
                fields = parse_team_metadata_schema(input_value["schema"])
            except ValueError:
                return {"failed": True}
            return {"fields": [{"key": field.key, "label": field.label} for field in fields]}
        case "registry":
            return registry_answer(input_value)
        case "validate":
            return validate_answer(input_value)
        case "sanitize":
            text = team_endpoints._sanitize_for_log(input_value["value"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L445-L451
            return {"text": text}
        case "edit_access":
            return edit_access_answer(input_value)
