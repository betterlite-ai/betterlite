import os
from typing import TypedDict, cast

from litellm.proxy._types import LiteLLM_JWTAuth
from litellm.proxy.auth.handle_jwt import JWTHandler
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.management_endpoints.types import CustomOpenID
from litellm.proxy.management_endpoints.ui_sso import (
    _decode_sso_token_claims,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L953-L963
    _merge_sso_token_claims,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L965-L974
    determine_role_from_groups,
    generic_response_convertor,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L1130-L1247
    normalize_email,
    process_sso_jwt_access_token,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L854-L951
)
from litellm.types.proxy.management_endpoints.ui_sso import RoleMappings, TeamMappings
from pydantic import JsonValue, TypeAdapter


class Settings(TypedDict, total=False):
    team_id_jwt_field: str | None
    team_ids_jwt_field: str | None
    team_id_default: str | None
    team_alias_jwt_field: str | None


class RoleMappingInput(TypedDict):
    provider: str
    group_claim: str
    default_role: str | None
    roles: dict[str, list[str]]


class TeamMappingInput(TypedDict):
    team_ids_jwt_field: str | None


class Identity(TypedDict):
    id: str | None
    email: str | None
    first_name: str | None
    last_name: str | None
    display_name: str | None
    picture: str | None
    provider: str | None
    team_ids: list[str]
    user_role: str | None
    extra_fields: dict[str, JsonValue] | None


class MergeCall(TypedDict):
    userinfo: dict[str, JsonValue]
    id_token: str | None
    access_token: str | None


class ProcessCall(TypedDict):
    access_token: str | None
    sso_settings: Settings | None
    identity: Identity | None
    role_mappings: RoleMappingInput | None


class ConvertCall(TypedDict):
    response: dict[str, JsonValue]
    jwt_settings: Settings
    sso_settings: Settings | None
    role_mappings: RoleMappingInput | None
    team_mappings: TeamMappingInput | None


class Input(TypedDict):
    attributes: dict[str, str]
    emails: list[str | None]
    groups: list[list[str]]
    role_mappings: RoleMappingInput | None
    merge: list[MergeCall]
    decode_tokens: list[str | None]
    process: list[ProcessCall]
    convert: ConvertCall


class Processed(TypedDict):
    identity: Identity | None
    payload: dict[str, JsonValue] | None


class Output(TypedDict):
    emails: list[str | None]
    roles: list[str | None]
    decoded: list[dict[str, JsonValue]]
    merged: list[dict[str, JsonValue]]
    processed: list[Processed]
    converted: Identity


IDENTITY = TypeAdapter(Identity)
CLAIMS = TypeAdapter(dict[str, JsonValue])


def build_handler(settings: Settings) -> JWTHandler:
    handler = JWTHandler()
    handler.update_environment(
        prisma_client=None,
        user_api_key_cache=UserApiKeyCache(),
        litellm_jwtauth=LiteLLM_JWTAuth(**settings),
        leeway=0,
    )
    return handler


def role_name(groups: list[str], mappings: RoleMappings | None) -> str | None:
    if mappings is None:
        return None
    role = determine_role_from_groups(groups, mappings)
    if role is None:
        return None
    return cast(str, role.value)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L841


def process_call(call: ProcessCall) -> Processed:
    result: CustomOpenID | None = None
    if call["identity"] is not None:
        result = CustomOpenID.model_validate(call["identity"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/types.py#L94-L97
    sso_jwt_handler: JWTHandler | None = None
    if call["sso_settings"] is not None:
        sso_jwt_handler = build_handler(call["sso_settings"])
    mappings: RoleMappings | None = None
    if call["role_mappings"] is not None:
        mappings = RoleMappings.model_validate(call["role_mappings"])
    payload = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L854-L951
        dict[str, JsonValue] | None,
        process_sso_jwt_access_token(
            access_token_str=call["access_token"],
            sso_jwt_handler=sso_jwt_handler,
            result=result,
            role_mappings=mappings,
        ),
    )
    updated: Identity | None = None
    if result is not None:
        updated = IDENTITY.validate_json(result.model_dump_json())
    return {"identity": updated, "payload": payload}


def convert_call(call: ConvertCall) -> Identity:
    sso_jwt_handler: JWTHandler | None = None
    if call["sso_settings"] is not None:
        sso_jwt_handler = build_handler(call["sso_settings"])
    mappings: RoleMappings | None = None
    if call["role_mappings"] is not None:
        mappings = RoleMappings.model_validate(call["role_mappings"])
    team_mappings: TeamMappings | None = None
    if call["team_mappings"] is not None:
        team_mappings = TeamMappings.model_validate(call["team_mappings"])
    converted = generic_response_convertor(
        response=call["response"],
        jwt_handler=build_handler(call["jwt_settings"]),
        sso_jwt_handler=sso_jwt_handler,
        role_mappings=mappings,
        team_mappings=team_mappings,
    )
    return IDENTITY.validate_json(converted.model_dump_json())


def run(input_value: Input) -> Output:
    for name, value in input_value["attributes"].items():
        os.environ[name] = value
    mappings: RoleMappings | None = None
    if input_value["role_mappings"] is not None:
        mappings = RoleMappings.model_validate(input_value["role_mappings"])
    return {
        "emails": [normalize_email(email) for email in input_value["emails"]],
        "roles": [role_name(groups, mappings) for groups in input_value["groups"]],
        "decoded": [
            CLAIMS.validate_python(_decode_sso_token_claims(token))
            for token in input_value["decode_tokens"]
        ],
        "merged": [
            CLAIMS.validate_python(
                _merge_sso_token_claims(call["userinfo"], call["id_token"], call["access_token"])
            )
            for call in input_value["merge"]
        ],
        "processed": [process_call(call) for call in input_value["process"]],
        "converted": convert_call(input_value["convert"]),
    }
