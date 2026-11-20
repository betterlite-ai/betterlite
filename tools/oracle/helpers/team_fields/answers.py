import asyncio
from collections.abc import Mapping
from typing import Final, Literal, TypedDict, assert_never, cast

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
    TeamMetadataValidatorRegistry,
    parse_team_metadata_schema,
    validate_team_metadata_if_configured,
)
from pydantic import BaseModel, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException as StarletteHTTPException

from .inputs import (
    Answer,
    EditAccessAnswer,
    EditAccessInput,
    Input,
    RegistryAnswer,
    RegistryInput,
    SanitizeAnswer,
    SanitizeInput,
    SchemaAnswer,
    SchemaInput,
    ValidateAnswer,
    ValidateInput,
)
from .validators import VALIDATOR_A, VALIDATOR_B, build_validator


class ErrorDetail(TypedDict):
    error: str


class ErrorEnvelope(BaseModel):
    status_code: int
    detail: ErrorDetail


ERROR_ENVELOPE: Final[TypeAdapter[ErrorEnvelope]] = TypeAdapter(ErrorEnvelope)


async def _configured(
    input_value: ValidateInput,
    caller: UserAPIKeyAuth,
    registry: TeamMetadataValidatorRegistry,
) -> None:
    await validate_team_metadata_if_configured(
        operation=input_value["operation"],
        metadata=cast("Mapping[str, JsonValue] | None", input_value["metadata"]),  # cast-ok: the case sends a non-dict value so the function's isinstance guard runs  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/team_metadata_validation.py#L171-L182
        existing_metadata=cast("Mapping[str, JsonValue] | None", input_value["existing_metadata"]),  # cast-ok: the case sends a non-dict value so the function's isinstance guard runs  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/team_metadata_validation.py#L171-L182
        team_id=input_value["team_id"],
        team_alias=input_value["team_alias"],
        user_api_key_dict=caller,
        registry=registry,
    )


def validate_answer(input_value: ValidateInput) -> ValidateAnswer:
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
        asyncio.run(_configured(input_value, caller, registry))
    except StarletteHTTPException as error:
        rejected = ERROR_ENVELOPE.validate_python(error, from_attributes=True)
        last = payloads[-1] if payloads else None
        return {"status": rejected.status_code, "error": rejected.detail["error"], "payload": last}
    return {"status": None, "error": None, "payload": payloads[-1] if payloads else None}


def schema_answer(input_value: SchemaInput) -> SchemaAnswer:
    try:
        fields = parse_team_metadata_schema(input_value["schema"])
    except ValueError:
        return {"failed": True}
    return {"fields": [{"key": field.key, "label": field.label} for field in fields]}


def registry_answer(input_value: RegistryInput) -> RegistryAnswer:
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


def sanitize_answer(input_value: SanitizeInput) -> SanitizeAnswer:
    text = team_endpoints._sanitize_for_log(input_value["value"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L445-L451
    return {"text": text}


def edit_access_answer(input_value: EditAccessInput) -> EditAccessAnswer:
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
            return schema_answer(input_value)
        case "registry":
            return registry_answer(input_value)
        case "validate":
            return validate_answer(input_value)
        case "sanitize":
            return sanitize_answer(input_value)
        case "edit_access":
            return edit_access_answer(input_value)
