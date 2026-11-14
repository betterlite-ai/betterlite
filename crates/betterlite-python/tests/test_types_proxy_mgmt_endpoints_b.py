import importlib
import json
from typing import Literal

import pytest
from litellm import types as litellm_types
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError

import betterlite.types.proxy.management_endpoints.scim_v2 as scim
import betterlite.types.proxy.management_endpoints.team_endpoints as team
import betterlite.types.proxy.management_endpoints.ui_sso as sso
import betterlite.types.proxy.public_endpoints.public_endpoints as public
import betterlite.types.proxy.ui_sso as ui

# litellm/types has no __init__.py, so pyright flags direct imports; load the modules by name
for module_name in (
    "litellm.types.proxy.management_endpoints.scim_v2",
    "litellm.types.proxy.management_endpoints.team_endpoints",
    "litellm.types.proxy.management_endpoints.ui_sso",
    "litellm.types.proxy.public_endpoints.public_endpoints",
    "litellm.types.proxy.ui_sso",
):
    importlib.import_module(module_name)

litellm_scim = litellm_types.proxy.management_endpoints.scim_v2
litellm_team = litellm_types.proxy.management_endpoints.team_endpoints
litellm_sso = litellm_types.proxy.management_endpoints.ui_sso
litellm_public = litellm_types.proxy.public_endpoints.public_endpoints
litellm_ui = litellm_types.proxy.ui_sso

ModelPair = tuple[type[BaseModel], type[BaseModel]]
PayloadPair = tuple[type[BaseModel], type[BaseModel], dict[str, JsonValue]]
Mode = Literal["validation", "serialization"]

SCIM_PAIRS: tuple[ModelPair, ...] = (
    (scim.LiteLLM_UserScimMetadata, litellm_scim.LiteLLM_UserScimMetadata),
    (scim.SCIMResource, litellm_scim.SCIMResource),
    (scim.SCIMUserName, litellm_scim.SCIMUserName),
    (scim.SCIMUserEmail, litellm_scim.SCIMUserEmail),
    (scim.SCIMUserGroup, litellm_scim.SCIMUserGroup),
    (scim.SCIMMultiValuedAttribute, litellm_scim.SCIMMultiValuedAttribute),
    (scim.SCIMUserManager, litellm_scim.SCIMUserManager),
    (scim.SCIMEnterpriseUser, litellm_scim.SCIMEnterpriseUser),
    (scim.SCIMUser, litellm_scim.SCIMUser),
    (scim.SCIMMember, litellm_scim.SCIMMember),
    (scim.SCIMGroup, litellm_scim.SCIMGroup),
    (scim.SCIMPlaceholderMergeResult, litellm_scim.SCIMPlaceholderMergeResult),
    (scim.SCIMListResponse, litellm_scim.SCIMListResponse),
    (scim.SCIMPatchOperation, litellm_scim.SCIMPatchOperation),
    (scim.SCIMPatchOp, litellm_scim.SCIMPatchOp),
    (scim.SCIMFeature, litellm_scim.SCIMFeature),
    (scim.SCIMServiceProviderConfig, litellm_scim.SCIMServiceProviderConfig),
    (scim.SCIMSchemaExtension, litellm_scim.SCIMSchemaExtension),
    (scim.SCIMResourceType, litellm_scim.SCIMResourceType),
    (scim.SCIMSchemaAttribute, litellm_scim.SCIMSchemaAttribute),
    (scim.SCIMSchema, litellm_scim.SCIMSchema),
)

# LiteLLM's SCIMUser model_serializer erases its properties, so only validation mode is compared
SCIM_SERIALIZATION_PAIRS: tuple[ModelPair, ...] = tuple(
    pair for pair in SCIM_PAIRS if pair[0] is not scim.SCIMUser
)

TEAM_PAIRS: tuple[ModelPair, ...] = (
    (team.GetTeamMemberPermissionsRequest, litellm_team.GetTeamMemberPermissionsRequest),
    (team.GetTeamMemberPermissionsResponse, litellm_team.GetTeamMemberPermissionsResponse),
    (team.UpdateTeamMemberPermissionsRequest, litellm_team.UpdateTeamMemberPermissionsRequest),
    (
        team.BulkUpdateTeamMemberPermissionsRequest,
        litellm_team.BulkUpdateTeamMemberPermissionsRequest,
    ),
    (
        team.BulkUpdateTeamMemberPermissionsResponse,
        litellm_team.BulkUpdateTeamMemberPermissionsResponse,
    ),
    (team.TeamListItem, litellm_team.TeamListItem),
    (team.TeamListResponse, litellm_team.TeamListResponse),
    (team.BulkTeamMemberAddRequest, litellm_team.BulkTeamMemberAddRequest),
    (team.TeamMemberAddResult, litellm_team.TeamMemberAddResult),
    (team.BulkTeamMemberAddResponse, litellm_team.BulkTeamMemberAddResponse),
    (team.TeamMemberRef, litellm_team.TeamMemberRef),
    (team.BulkTeamMemberDeleteRequest, litellm_team.BulkTeamMemberDeleteRequest),
    (team.TeamMemberDeleteResult, litellm_team.TeamMemberDeleteResult),
    (team.BulkTeamMemberDeleteResponse, litellm_team.BulkTeamMemberDeleteResponse),
    (team.TeamMemberBudgetPatch, litellm_team.TeamMemberBudgetPatch),
    (
        team.BulkTeamMemberBudgetUpdateRequest,
        litellm_team.BulkTeamMemberBudgetUpdateRequest,
    ),
    (team.TeamMemberBudgetUpdateResult, litellm_team.TeamMemberBudgetUpdateResult),
    (
        team.BulkTeamMemberBudgetUpdateResponse,
        litellm_team.BulkTeamMemberBudgetUpdateResponse,
    ),
    (team.TeamMemberInfoResponse, litellm_team.TeamMemberInfoResponse),
    (team.TeamMetadataFieldSchema, litellm_team.TeamMetadataFieldSchema),
    (team.TeamMetadataSchemaResponse, litellm_team.TeamMetadataSchemaResponse),
    (team.TeamUserSpendRow, litellm_team.TeamUserSpendRow),
    (team.TeamUserSpendResponse, litellm_team.TeamUserSpendResponse),
)

SSO_PAIRS: tuple[ModelPair, ...] = (
    (sso.LiteLLM_UpperboundKeyGenerateParams, litellm_sso.LiteLLM_UpperboundKeyGenerateParams),
    (sso.AccessControl_UI_AccessMode, litellm_sso.AccessControl_UI_AccessMode),
    (sso.RoleMappings, litellm_sso.RoleMappings),
    (sso.TeamMappings, litellm_sso.TeamMappings),
    (sso.SSOConfig, litellm_sso.SSOConfig),
    (sso.DefaultTeamSSOParams, litellm_sso.DefaultTeamSSOParams),
)

PUBLIC_PAIRS: tuple[ModelPair, ...] = (
    (public.PublicModelHubInfo, litellm_public.PublicModelHubInfo),
    (public.ProviderCredentialField, litellm_public.ProviderCredentialField),
    (public.ProviderCreateInfo, litellm_public.ProviderCreateInfo),
    (public.AgentCredentialField, litellm_public.AgentCredentialField),
    (public.AgentCreateInfo, litellm_public.AgentCreateInfo),
    (public.EndpointProvider, litellm_public.EndpointProvider),
    (public.SupportedEndpoint, litellm_public.SupportedEndpoint),
    (public.SupportedEndpointsResponse, litellm_public.SupportedEndpointsResponse),
    (public.AutoRouterPresetTiers, litellm_public.AutoRouterPresetTiers),
    (public.AutoRouterPresetConfig, litellm_public.AutoRouterPresetConfig),
    (public.AutoRouterPresetRecord, litellm_public.AutoRouterPresetRecord),
    (public.ComplexityScorerDefaults, litellm_public.ComplexityScorerDefaults),
)

SCIM_WIRE_PAYLOADS: tuple[PayloadPair, ...] = (
    (scim.SCIMGroup, litellm_scim.SCIMGroup, {"schemas": ["s"], "displayName": "g"}),
    (
        scim.SCIMGroup,
        litellm_scim.SCIMGroup,
        {
            "schemas": ["s"],
            "displayName": "g",
            "members": [{"value": "u", "display": "alice", "type": "direct"}],
        },
    ),
    (scim.SCIMListResponse, litellm_scim.SCIMListResponse, {"totalResults": 0, "Resources": []}),
    (
        scim.SCIMListResponse,
        litellm_scim.SCIMListResponse,
        {
            "totalResults": 1,
            "startIndex": 2,
            "itemsPerPage": 3,
            "Resources": [{"schemas": ["s"], "userName": "bob"}],
        },
    ),
    (
        scim.SCIMPatchOp,
        litellm_scim.SCIMPatchOp,
        {"Operations": [{"op": "add", "path": "active", "value": True}]},
    ),
    (
        scim.SCIMServiceProviderConfig,
        litellm_scim.SCIMServiceProviderConfig,
        {"authenticationSchemes": [{"type": "httpbasic"}], "meta": {"resourceType": "Config"}},
    ),
    (
        scim.SCIMResourceType,
        litellm_scim.SCIMResourceType,
        {
            "id": "r",
            "name": "n",
            "endpoint": "/Users",
            "schema_": "urn:x",
            "schemaExtensions": [{"schema_": "urn:e", "required": False}],
        },
    ),
    (
        scim.SCIMSchema,
        litellm_scim.SCIMSchema,
        {
            "id": "s",
            "name": "n",
            "attributes": [
                {"name": "a", "type": "complex", "multiValued": True, "subAttributes": []},
                {"name": "b", "type": "string"},
            ],
        },
    ),
)

TEAM_VALID_PAYLOADS: tuple[PayloadPair, ...] = (
    (team.TeamMemberRef, litellm_team.TeamMemberRef, {"user_id": "u"}),
    (
        team.TeamMemberBudgetPatch,
        litellm_team.TeamMemberBudgetPatch,
        {"user_email": "e@x", "max_budget_in_team": 1.0, "allowed_models": ["m"]},
    ),
    (team.TeamMemberAddResult, litellm_team.TeamMemberAddResult, {"success": True, "error": None}),
    (
        team.BulkTeamMemberDeleteResponse,
        litellm_team.BulkTeamMemberDeleteResponse,
        {"data": [{"success": True}, {"user_id": "u", "success": False}]},
    ),
    (
        team.BulkTeamMemberBudgetUpdateResponse,
        litellm_team.BulkTeamMemberBudgetUpdateResponse,
        {"data": [{"success": True, "max_budget_source": "team_default"}]},
    ),
    (team.TeamMetadataFieldSchema, litellm_team.TeamMetadataFieldSchema, {"key": "k"}),
    (
        team.TeamMetadataSchemaResponse,
        litellm_team.TeamMetadataSchemaResponse,
        {"fields": [{"key": "k", "label": "l"}]},
    ),
    (team.TeamUserSpendRow, litellm_team.TeamUserSpendRow, {"team_id": "t", "user_id": "u"}),
    (
        team.TeamUserSpendResponse,
        litellm_team.TeamUserSpendResponse,
        {"start_date": "a", "end_date": "b", "results": []},
    ),
)

TEAM_REJECTIONS: tuple[PayloadPair, ...] = (
    (team.TeamMemberRef, litellm_team.TeamMemberRef, {}),
    (team.TeamMemberRef, litellm_team.TeamMemberRef, {"user_id": "u", "user_email": "e"}),
    (team.TeamMemberRef, litellm_team.TeamMemberRef, {"user_id": "u", "extra": 1}),
    (
        team.BulkTeamMemberDeleteRequest,
        litellm_team.BulkTeamMemberDeleteRequest,
        {"members": []},
    ),
    (
        team.BulkTeamMemberBudgetUpdateRequest,
        litellm_team.BulkTeamMemberBudgetUpdateRequest,
        {"members": []},
    ),
    (
        team.TeamMetadataFieldSchema,
        litellm_team.TeamMetadataFieldSchema,
        {"key": ""},
    ),
    (
        team.TeamMetadataFieldSchema,
        litellm_team.TeamMetadataFieldSchema,
        {"key": "k", "extra": 1},
    ),
    (
        team.TeamMemberBudgetUpdateResult,
        litellm_team.TeamMemberBudgetUpdateResult,
        {"success": True, "max_budget_source": "bogus"},
    ),
)

OVER_LIMIT_DELETES: dict[str, JsonValue] = {
    "members": [{"user_id": "u"}] * (team.MAX_BULK_TEAM_MEMBER_DELETES + 1)
}

OVER_LIMIT_BUDGET_UPDATES: dict[str, JsonValue] = {
    "members": [{"user_id": "u"}] * (team.MAX_BULK_TEAM_MEMBER_BUDGET_UPDATES + 1)
}


def schema_properties(schema: dict[str, JsonValue]) -> dict[str, JsonValue]:
    match schema["properties"]:
        case dict() as properties:
            return properties
        case None | str() | int() | float() | bool() | list():
            raise TypeError("schema has no properties")


def required_names(schema: dict[str, JsonValue]) -> list[str]:
    match schema.get("required"):
        case None:
            return []
        case list() as names:
            return sorted(json.dumps(name) for name in names)
        case str() | int() | float() | bool() | dict():
            raise TypeError("required is not a list")


def defaults_of(properties: dict[str, JsonValue]) -> dict[str, JsonValue]:
    defaults: dict[str, JsonValue] = {}
    for name, field in properties.items():
        match field:
            case dict():
                if "default" in field:
                    defaults[name] = field["default"]
            case None | str() | int() | float() | bool() | list():
                raise TypeError("property is not an object")
    return defaults


def collect_literals(node: JsonValue, found: set[str]) -> None:
    match node:
        case dict():
            for key, item in node.items():
                if key in ("enum", "const"):
                    found.add(json.dumps(item, sort_keys=True))
                else:
                    collect_literals(item, found)
        case list():
            for item in node:
                collect_literals(item, found)
        case None | str() | int() | float() | bool():
            return


def literals_of(properties: dict[str, JsonValue]) -> list[tuple[str, list[str]]]:
    found: list[tuple[str, list[str]]] = []
    for name, field in properties.items():
        values: set[str] = set()
        collect_literals(field, values)
        found.append((name, sorted(values)))
    return sorted(found)


def resolve_ref(schema: dict[str, JsonValue]) -> dict[str, JsonValue]:
    match schema.get("$ref"):
        case None:
            return schema
        case str() as reference:
            match schema["$defs"]:
                case dict() as definitions:
                    match definitions[reference.rsplit("/", 1)[-1]]:
                        case dict() as resolved:
                            return resolved
                        case None | str() | int() | float() | bool() | list():
                            raise TypeError("definition is not an object")
                case None | str() | int() | float() | bool() | list():
                    raise TypeError("$defs is not an object")
        case int() | float() | bool() | list() | dict():
            raise TypeError("$ref is not a string")


def schema_view(schema: dict[str, JsonValue]) -> dict[str, JsonValue]:
    resolved = resolve_ref(schema)
    properties = schema_properties(resolved)
    return {
        "keys": sorted(properties),
        "required": required_names(resolved),
        "defaults": defaults_of(properties),
        "literals": literals_of(properties),
    }


def model_view(model: type[BaseModel], mode: Mode) -> dict[str, JsonValue]:
    schema: dict[str, JsonValue] = model.model_json_schema(mode=mode)
    return schema_view(schema)


def rejects(model: type[BaseModel], payload: dict[str, JsonValue]) -> bool:
    try:
        model.model_validate(payload)
    except ValidationError:
        return True
    else:
        return False


def assert_views_match(pairs: tuple[ModelPair, ...], mode: Mode) -> None:
    for ours, reference in pairs:
        assert model_view(ours, mode) == model_view(reference, mode), (ours.__name__, mode)


def test_names_scim_models() -> None:
    assert_views_match(SCIM_PAIRS, "validation")
    assert_views_match(SCIM_SERIALIZATION_PAIRS, "serialization")


def test_names_team_models() -> None:
    assert_views_match(TEAM_PAIRS, "validation")
    assert_views_match(TEAM_PAIRS, "serialization")


def test_names_sso_models() -> None:
    assert_views_match(SSO_PAIRS, "validation")
    assert_views_match(SSO_PAIRS, "serialization")


def test_names_public_models() -> None:
    assert_views_match(PUBLIC_PAIRS, "validation")
    assert_views_match(PUBLIC_PAIRS, "serialization")


def test_names_sso_typed_dicts() -> None:
    our_group = TypeAdapter(sso.MicrosoftGraphAPIUserGroupDirectoryObject)
    their_group = TypeAdapter(litellm_sso.MicrosoftGraphAPIUserGroupDirectoryObject)
    our_response = TypeAdapter(sso.MicrosoftGraphAPIUserGroupResponse)
    their_response = TypeAdapter(litellm_sso.MicrosoftGraphAPIUserGroupResponse)
    our_principal = TypeAdapter(sso.MicrosoftServicePrincipalTeam)
    their_principal = TypeAdapter(litellm_sso.MicrosoftServicePrincipalTeam)
    for ours, theirs in (
        (our_group, their_group),
        (our_response, their_response),
        (our_principal, their_principal),
    ):
        assert schema_view(ours.json_schema(mode="validation")) == schema_view(
            theirs.json_schema(mode="validation")
        )


def test_names_ui_typed_dicts() -> None:
    our_token = TypeAdapter(ui.ReturnedUITokenObject)
    their_token = TypeAdapter(litellm_ui.ReturnedUITokenObject)
    our_openid = TypeAdapter(ui.ParsedOpenIDResult)
    their_openid = TypeAdapter(litellm_ui.ParsedOpenIDResult)
    for ours, theirs in ((our_token, their_token), (our_openid, their_openid)):
        assert schema_view(ours.json_schema(mode="validation")) == schema_view(
            theirs.json_schema(mode="validation")
        )


def test_names_scim_constants() -> None:
    assert scim.SCIM_ENTERPRISE_USER_SCHEMA == litellm_scim.SCIM_ENTERPRISE_USER_SCHEMA
    assert scim.SCIM_ENTERPRISE_METADATA_KEY == litellm_scim.SCIM_ENTERPRISE_METADATA_KEY
    assert scim.SCIM_ENTITLEMENTS_METADATA_KEY == litellm_scim.SCIM_ENTITLEMENTS_METADATA_KEY
    assert scim.SCIM_ROLES_METADATA_KEY == litellm_scim.SCIM_ROLES_METADATA_KEY
    assert scim.SCIM_MANAGED_TEAM_METADATA_KEY == litellm_scim.SCIM_MANAGED_TEAM_METADATA_KEY
    assert scim.SCIM_TEAM_DATA_METADATA_KEY == litellm_scim.SCIM_TEAM_DATA_METADATA_KEY
    assert (
        scim.SCIM_MULTI_VALUED_ATTRIBUTE_METADATA_KEYS
        == litellm_scim.SCIM_MULTI_VALUED_ATTRIBUTE_METADATA_KEYS
    )


def test_names_team_constants() -> None:
    assert team.TeamIdSearchMatch == litellm_team.TeamIdSearchMatch
    assert team.MAX_BULK_TEAM_MEMBER_DELETES == litellm_team.MAX_BULK_TEAM_MEMBER_DELETES
    assert (
        team.MAX_BULK_TEAM_MEMBER_BUDGET_UPDATES
        == litellm_team.MAX_BULK_TEAM_MEMBER_BUDGET_UPDATES
    )


def test_names_scim_wire_dumps() -> None:
    for ours, reference, payload in SCIM_WIRE_PAYLOADS:
        our_model = ours.model_validate(payload)
        reference_model = reference.model_validate(payload)
        assert our_model.model_dump(by_alias=True) == reference_model.model_dump(by_alias=True)
        assert our_model.model_dump_json(by_alias=True) == reference_model.model_dump_json(
            by_alias=True
        )


def test_names_scim_absent_blocks() -> None:
    payload: dict[str, JsonValue] = {"schemas": ["s"], "userName": "alice"}
    our_dump = scim.SCIMUser.model_validate(payload).model_dump(by_alias=True)
    reference_dump = litellm_scim.SCIMUser.model_validate(payload).model_dump(by_alias=True)
    assert our_dump == reference_dump
    assert "entitlements" not in our_dump
    assert "roles" not in our_dump
    assert scim.SCIM_ENTERPRISE_USER_SCHEMA not in our_dump


def test_names_scim_deviations() -> None:
    email: dict[str, JsonValue] = {"value": "not-an-email"}
    assert not rejects(scim.SCIMUserEmail, email)
    assert rejects(litellm_scim.SCIMUserEmail, email)
    assert "user_name" in scim.SCIMUser.model_fields
    assert "userName" in litellm_scim.SCIMUser.model_fields
    snake_case: dict[str, JsonValue] = {"schemas": ["s"], "user_name": "alice"}
    our_dump = scim.SCIMUser.model_validate(snake_case).model_dump(by_alias=True)
    their_dump = litellm_scim.SCIMUser.model_validate(snake_case).model_dump(by_alias=True)
    assert our_dump["userName"] == "alice"
    assert their_dump["userName"] is None
    our_schema: dict[str, JsonValue] = scim.SCIMUser.model_json_schema(mode="serialization")
    their_schema: dict[str, JsonValue] = litellm_scim.SCIMUser.model_json_schema(
        mode="serialization"
    )
    assert "properties" in our_schema
    assert "properties" not in their_schema


def test_names_scim_user_dump() -> None:
    payload: dict[str, JsonValue] = {
        "schemas": ["s"],
        "userName": "alice",
        "active": False,
        "emails": [{"value": "a@b.c", "primary": True}],
        "entitlements": [{"value": "e1"}],
        "roles": ["bare-role"],
        scim.SCIM_ENTERPRISE_USER_SCHEMA: {
            "employeeNumber": "7",
            "manager": {"value": "m", "$ref": "https://x"},
        },
    }
    our_model = scim.SCIMUser.model_validate(payload)
    reference_model = litellm_scim.SCIMUser.model_validate(payload)
    assert our_model.model_dump(by_alias=True) == reference_model.model_dump(by_alias=True)
    assert our_model.model_dump_json(by_alias=True) == reference_model.model_dump_json(
        by_alias=True
    )


def test_names_scim_coercions() -> None:
    our_attribute = scim.SCIMMultiValuedAttribute.model_validate("bare")
    reference_attribute = litellm_scim.SCIMMultiValuedAttribute.model_validate("bare")
    assert our_attribute.model_dump() == reference_attribute.model_dump()
    our_list = scim.SCIM_MULTI_VALUED_LIST_ADAPTER.validate_python(["bare", {"value": "v"}])
    reference_list = litellm_scim.SCIM_MULTI_VALUED_LIST_ADAPTER.validate_python(
        ["bare", {"value": "v"}]
    )
    assert [item.model_dump() for item in our_list] == [
        item.model_dump() for item in reference_list
    ]
    member: dict[str, JsonValue] = {"value": "u", "type": 5}
    assert (
        scim.SCIMMember.model_validate(member).model_dump()
        == litellm_scim.SCIMMember.model_validate(member).model_dump()
    )


def test_names_scim_patch_ops() -> None:
    for payload in (
        {"op": "ADD", "path": "active", "value": True},
        {"op": "Remove", "path": "x"},
        {"op": "replace", "value": None},
    ):
        assert (
            scim.SCIMPatchOperation.model_validate(payload).model_dump()
            == litellm_scim.SCIMPatchOperation.model_validate(payload).model_dump()
        )
    assert scim.SCIMPatchOperation.model_validate({"op": "ADD"}).op == "add"
    bogus: dict[str, JsonValue] = {"op": "bogus"}
    assert rejects(scim.SCIMPatchOperation, bogus)
    assert rejects(litellm_scim.SCIMPatchOperation, bogus)


def test_names_team_valid_payloads() -> None:
    for ours, reference, payload in TEAM_VALID_PAYLOADS:
        assert ours.model_validate(payload).model_dump() == reference.model_validate(
            payload
        ).model_dump()
    add_payload: dict[str, JsonValue] = {
        "team_id": "t",
        "members": [{"role": "user", "user_id": "u"}],
        "all_users": None,
        "max_budget_in_team": 1.0,
    }
    our_add = team.BulkTeamMemberAddRequest.model_validate(add_payload).model_dump()
    reference_add = litellm_team.BulkTeamMemberAddRequest.model_validate(add_payload).model_dump()
    assert sorted(our_add) == sorted(reference_add)
    info_payload: dict[str, JsonValue] = {
        "user_id": "u",
        "team_id": "t",
        "litellm_budget_table": {"budget_id": "b"},
        "role": "user",
    }
    our_info = team.TeamMemberInfoResponse.model_validate(info_payload).model_dump()
    reference_info = litellm_team.TeamMemberInfoResponse.model_validate(info_payload).model_dump()
    assert sorted(our_info) == sorted(reference_info)


@pytest.mark.parametrize(("ours", "reference", "payload"), TEAM_REJECTIONS)
def test_names_team_rejections(
    ours: type[BaseModel], reference: type[BaseModel], payload: dict[str, JsonValue]
) -> None:
    assert rejects(ours, payload)
    assert rejects(reference, payload)


def test_names_team_member_limit() -> None:
    assert rejects(team.BulkTeamMemberDeleteRequest, OVER_LIMIT_DELETES)
    assert rejects(litellm_team.BulkTeamMemberDeleteRequest, OVER_LIMIT_DELETES)
    assert rejects(team.BulkTeamMemberBudgetUpdateRequest, OVER_LIMIT_BUDGET_UPDATES)
    assert rejects(litellm_team.BulkTeamMemberBudgetUpdateRequest, OVER_LIMIT_BUDGET_UPDATES)
    at_limit: dict[str, JsonValue] = {
        "members": [{"user_id": "u"}] * team.MAX_BULK_TEAM_MEMBER_DELETES
    }
    assert not rejects(team.BulkTeamMemberDeleteRequest, at_limit)
    assert not rejects(litellm_team.BulkTeamMemberDeleteRequest, at_limit)
    budget_at_limit: dict[str, JsonValue] = {
        "members": [{"user_id": "u"}] * team.MAX_BULK_TEAM_MEMBER_BUDGET_UPDATES
    }
    assert not rejects(team.BulkTeamMemberBudgetUpdateRequest, budget_at_limit)
    assert not rejects(litellm_team.BulkTeamMemberBudgetUpdateRequest, budget_at_limit)


def test_names_team_deviations() -> None:
    budget_patch: dict[str, JsonValue] = {"user_id": "u", "budget_duration": "bogus"}
    assert not rejects(team.TeamMemberBudgetPatch, budget_patch)
    assert rejects(litellm_team.TeamMemberBudgetPatch, budget_patch)
    permissions: dict[str, JsonValue] = {"permissions": ["/bogus"]}
    assert not rejects(team.BulkUpdateTeamMemberPermissionsRequest, permissions)
    assert rejects(litellm_team.BulkUpdateTeamMemberPermissionsRequest, permissions)
    member: dict[str, JsonValue] = {
        "team_id": "t",
        "members_with_roles": [{"user_id": "u", "role": "user"}],
    }
    our_member = team.TeamListItem.model_validate(member).model_dump()
    their_member = litellm_team.TeamListItem.model_validate(member).model_dump()
    assert our_member["members_with_roles"] == [{"user_id": "u", "role": "user"}]
    assert their_member["members_with_roles"] == [
        {"user_id": "u", "user_email": None, "role": "user"}
    ]


def test_names_sso_deviations() -> None:
    permissions: dict[str, JsonValue] = {"team_member_permissions": ["/bogus"]}
    assert not rejects(sso.DefaultTeamSSOParams, permissions)
    assert rejects(litellm_sso.DefaultTeamSSOParams, permissions)
    role: dict[str, JsonValue] = {"provider": "p", "group_claim": "g", "default_role": "bogus"}
    assert not rejects(sso.RoleMappings, role)
    assert rejects(litellm_sso.RoleMappings, role)


def test_names_public_presets() -> None:
    tiers: dict[str, JsonValue] = {"SIMPLE": ["a"], "MEDIUM": [], "COMPLEX": [], "REASONING": ["r"]}
    assert (
        public.AutoRouterPresetTiers.model_validate(tiers).model_dump()
        == litellm_public.AutoRouterPresetTiers.model_validate(tiers).model_dump()
    )
    extra_tiers: dict[str, JsonValue] = tiers | {"UNKNOWN": []}
    assert rejects(public.AutoRouterPresetTiers, extra_tiers)
    assert rejects(litellm_public.AutoRouterPresetTiers, extra_tiers)
    config: dict[str, JsonValue] = {"tiers": tiers, "future_field": {"x": 1}}
    assert (
        public.AutoRouterPresetConfig.model_validate(config).model_dump()
        == litellm_public.AutoRouterPresetConfig.model_validate(config).model_dump()
    )
    record: dict[str, JsonValue] = {
        "label": "l",
        "description": "d",
        "complexity_router_config": config,
    }
    assert (
        public.AutoRouterPresetRecord.model_validate(record).model_dump()
        == litellm_public.AutoRouterPresetRecord.model_validate(record).model_dump()
    )
