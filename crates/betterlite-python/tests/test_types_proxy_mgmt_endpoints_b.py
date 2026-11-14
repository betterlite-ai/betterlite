import json
import subprocess
import sys
from typing import Final, TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter

LITELLM: Final = "litellm"
BETTERLITE: Final = "betterlite"

PROBE: Final = """
import importlib, inspect, json, sys
from pydantic import BaseModel, TypeAdapter, ValidationError

root = sys.argv[1]
BASE = root + ".types.proxy."


def load(name):
    return importlib.import_module(BASE + name)


def resolve(schema):
    reference = schema.get("$ref")
    if reference is None:
        return schema
    return schema["$defs"][reference.rsplit("/", 1)[-1]]


def collect(node, found):
    if isinstance(node, dict):
        for key, item in node.items():
            if key in ("enum", "const"):
                found.add(json.dumps(item, sort_keys=True))
            else:
                collect(item, found)
    elif isinstance(node, list):
        for item in node:
            collect(item, found)


def view(schema):
    schema = resolve(schema)
    properties = schema["properties"]
    defaults = {}
    literals = []
    for name, field in properties.items():
        if "default" in field:
            defaults[name] = field["default"]
        found = set()
        collect(field, found)
        literals.append([name, sorted(found)])
    return {
        "keys": sorted(properties),
        "required": sorted(schema.get("required", [])),
        "defaults": defaults,
        "literals": sorted(literals),
    }


def entry(obj, serialization):
    schema = TypeAdapter(obj).json_schema(mode="validation")
    result = {"validation": view(schema)}
    if serialization:
        result["serialization"] = view(TypeAdapter(obj).json_schema(mode="serialization"))
    return result


def descriptions(obj):
    schema = resolve(TypeAdapter(obj).json_schema(mode="validation"))
    fields = schema["properties"]
    return {
        "root": schema.get("description"),
        "fields": {key: field.get("description") for key, field in fields.items()},
    }


def attempts(call):
    try:
        call()
    except ValidationError as error:
        return {
            "raised": "ValidationError",
            "errors": [[item["type"], list(item["loc"])] for item in error.errors()],
        }
    except AttributeError:
        return {"raised": "AttributeError"}
    return {"raised": None}


def dump(model, payload):
    instance = model.model_validate(payload)
    return {
        "json": instance.model_dump(mode="json"),
        "wire": sorted(instance.model_dump(by_alias=True)),
    }


def dump_alias(model, payload):
    instance = model.model_validate(payload)
    return {
        "json": instance.model_dump(mode="json", by_alias=True),
        "wire": sorted(instance.model_dump(by_alias=True)),
    }


def scenario_scim_wire():
    module = load("management_endpoints.scim_v2")
    cases = {
        "group_display": (module.SCIMGroup, {"schemas": ["s"], "displayName": "g"}),
        "group_members": (
            module.SCIMGroup,
            {
                "schemas": ["s"],
                "displayName": "g",
                "members": [{"value": "u", "display": "alice", "type": "direct"}],
            },
        ),
        "list_empty": (module.SCIMListResponse, {"totalResults": 0, "Resources": []}),
        "list_items": (
            module.SCIMListResponse,
            {
                "totalResults": 1,
                "startIndex": 2,
                "itemsPerPage": 3,
                "Resources": [{"schemas": ["s"], "userName": "bob"}],
            },
        ),
        "patch": (
            module.SCIMPatchOp,
            {"Operations": [{"op": "add", "path": "active", "value": True}]},
        ),
        "config": (
            module.SCIMServiceProviderConfig,
            {
                "authenticationSchemes": [{"type": "httpbasic"}],
                "meta": {"resourceType": "Config"},
            },
        ),
        "resource_type": (
            module.SCIMResourceType,
            {
                "id": "r",
                "name": "n",
                "endpoint": "/Users",
                "schema_": "urn:x",
                "schemaExtensions": [{"schema_": "urn:e", "required": False}],
            },
        ),
        "schema": (
            module.SCIMSchema,
            {
                "id": "s",
                "name": "n",
                "attributes": [
                    {"name": "a", "type": "complex", "multiValued": True, "subAttributes": []},
                    {"name": "b", "type": "string"},
                ],
            },
        ),
    }
    return {name: dump(model, payload) for name, (model, payload) in cases.items()}


def scenario_scim_overrides():
    module = load("management_endpoints.scim_v2")
    resource_type = module.SCIMResourceType.model_validate(
        {
            "id": "User",
            "name": "User",
            "endpoint": "/Users",
            "schema_": "urn:ietf:params:scim:schemas:core:2.0:User",
            "meta": {"resourceType": "ResourceType"},
        }
    )
    extension = module.SCIMSchemaExtension(schema_="urn:e", required=False)
    attribute = module.SCIMSchemaAttribute(name="a", type="string")
    return {
        "resource_default": resource_type.model_dump(mode="json"),
        "resource_flat": resource_type.model_dump(mode="json", by_alias=False),
        "extension_default": extension.model_dump(mode="json"),
        "attribute_default": attribute.model_dump(mode="json"),
    }


def scenario_scim_absent():
    module = load("management_endpoints.scim_v2")
    payload = {"schemas": ["s"], "userName": "alice"}
    user = module.SCIMUser.model_validate(payload)
    return {"wire": sorted(user.model_dump(by_alias=True))}


def scenario_scim_deviations():
    module = load("management_endpoints.scim_v2")
    snake_case = {"schemas": ["s"], "user_name": "alice"}
    serialization = module.SCIMUser.model_json_schema(mode="serialization")
    return {
        "metadata_name": module.LiteLLM_UserScimMetadata.__name__,
        "bad_email": attempts(
            lambda: module.SCIMUserEmail.model_validate({"value": "not-an-email"})
        ),
        "fields": sorted(module.SCIMUser.model_fields),
        "snake_case": module.SCIMUser.model_validate(snake_case).model_dump(
            mode="json", by_alias=True
        ),
        "serialization_properties": "properties" in serialization,
    }


def scenario_scim_email():
    module = load("management_endpoints.scim_v2")
    return {
        address: dump(module.SCIMUserEmail, {"value": address})
        for address in ("Alice@EXAMPLE.COM", "  bob@x.io ")
    }


def scenario_scim_json():
    module = load("management_endpoints.scim_v2")
    payload = {"schemas": ["s"], "meta": {"tags": {"x"}}}
    return {"set_meta": attempts(lambda: module.SCIMResource.model_validate(payload))}


def scenario_scim_serialization():
    module = load("management_endpoints.scim_v2")
    schema = TypeAdapter(module.SCIMUser).json_schema(mode="serialization")
    return {"has_properties": "properties" in schema}


def scenario_scim_user_dump():
    module = load("management_endpoints.scim_v2")
    payload = {
        "schemas": ["s"],
        "userName": "alice",
        "active": False,
        "emails": [{"value": "a@b.c", "primary": True}],
        "entitlements": [{"value": "e1"}],
        "roles": ["bare-role"],
        "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User": {
            "employeeNumber": "7",
            "manager": {"value": "m", "$ref": "https://x"},
        },
    }
    return {"user": dump_alias(module.SCIMUser, payload)}


def scenario_scim_coercions():
    module = load("management_endpoints.scim_v2")
    attribute = module.SCIMMultiValuedAttribute.model_validate("bare")
    listed = module.SCIM_MULTI_VALUED_LIST_ADAPTER.validate_python(["bare", {"value": "v"}])
    member = module.SCIMMember.model_validate({"value": "u", "type": 5})
    return {
        "attribute": attribute.model_dump(mode="json"),
        "list": [item.model_dump(mode="json") for item in listed],
        "member": member.model_dump(mode="json"),
    }


def scenario_scim_patches():
    module = load("management_endpoints.scim_v2")
    cases = {
        "upper": {"op": "ADD", "path": "active", "value": True},
        "mixed": {"op": "Remove", "path": "x"},
        "null_value": {"op": "replace", "value": None},
    }
    result = {name: dump(module.SCIMPatchOperation, payload) for name, payload in cases.items()}
    result["normalized"] = module.SCIMPatchOperation.model_validate({"op": "ADD"}).op
    result["bogus"] = attempts(lambda: module.SCIMPatchOperation.model_validate({"op": "bogus"}))
    return result


def scenario_team_valid():
    module = load("management_endpoints.team_endpoints")
    cases = {
        "member_ref": (module.TeamMemberRef, {"user_id": "u"}),
        "budget_patch": (
            module.TeamMemberBudgetPatch,
            {"user_email": "e@x", "max_budget_in_team": 1.0, "allowed_models": ["m"]},
        ),
        "add_result": (module.TeamMemberAddResult, {"success": True, "error": None}),
        "delete_response": (
            module.BulkTeamMemberDeleteResponse,
            {"data": [{"success": True}, {"user_id": "u", "success": False}]},
        ),
        "budget_response": (
            module.BulkTeamMemberBudgetUpdateResponse,
            {"data": [{"success": True, "max_budget_source": "team_default"}]},
        ),
        "metadata_field": (module.TeamMetadataFieldSchema, {"key": "k"}),
        "metadata_response": (
            module.TeamMetadataSchemaResponse,
            {"fields": [{"key": "k", "label": "l"}]},
        ),
        "spend_row": (module.TeamUserSpendRow, {"team_id": "t", "user_id": "u"}),
        "spend_response": (
            module.TeamUserSpendResponse,
            {"start_date": "a", "end_date": "b", "results": []},
        ),
    }
    result = {name: dump(model, payload) for name, (model, payload) in cases.items()}
    add_request = module.BulkTeamMemberAddRequest.model_validate(
        {
            "team_id": "t",
            "members": [{"role": "user", "user_id": "u"}],
            "all_users": None,
            "max_budget_in_team": 1.0,
        }
    )
    info = module.TeamMemberInfoResponse.model_validate(
        {
            "user_id": "u",
            "team_id": "t",
            "litellm_budget_table": {"budget_id": "b"},
            "role": "user",
        }
    )
    result["add_keys"] = sorted(add_request.model_dump())
    result["info_keys"] = sorted(info.model_dump())
    return result


def scenario_team_lists():
    module = load("management_endpoints.team_endpoints")
    payload = {
        "teams": [
            {"team_id": "d", "deleted_at": "2024-01-01T00:00:00Z", "deleted_by": "u"},
            {"team_id": "p", "team_alias": "a"},
        ],
        "total": 2,
        "page": 1,
        "page_size": 10,
        "total_pages": 1,
    }
    return {"list": dump(module.TeamListResponse, payload)}


def scenario_team_rejections():
    module = load("management_endpoints.team_endpoints")
    cases = {
        "missing_ids": (module.TeamMemberRef, {}),
        "both_ids": (module.TeamMemberRef, {"user_id": "u", "user_email": "e"}),
        "extra_field": (module.TeamMemberRef, {"user_id": "u", "extra": 1}),
        "delete_empty": (module.BulkTeamMemberDeleteRequest, {"members": []}),
        "budget_empty": (module.BulkTeamMemberBudgetUpdateRequest, {"members": []}),
        "empty_key": (module.TeamMetadataFieldSchema, {"key": ""}),
        "key_extra": (module.TeamMetadataFieldSchema, {"key": "k", "extra": 1}),
        "bad_source": (
            module.TeamMemberBudgetUpdateResult,
            {"success": True, "max_budget_source": "bogus"},
        ),
    }
    return {
        name: attempts(lambda: model.model_validate(payload))
        for name, (model, payload) in cases.items()
    }


def scenario_team_limits():
    module = load("management_endpoints.team_endpoints")
    row = [{"user_id": "u"}]
    deletes = module.MAX_BULK_TEAM_MEMBER_DELETES
    updates = module.MAX_BULK_TEAM_MEMBER_BUDGET_UPDATES
    return {
        "over_deletes": attempts(
            lambda: module.BulkTeamMemberDeleteRequest.model_validate(
                {"members": row * (deletes + 1)}
            )
        ),
        "at_deletes": attempts(
            lambda: module.BulkTeamMemberDeleteRequest.model_validate({"members": row * deletes})
        ),
        "over_updates": attempts(
            lambda: module.BulkTeamMemberBudgetUpdateRequest.model_validate(
                {"members": row * (updates + 1)}
            )
        ),
        "at_updates": attempts(
            lambda: module.BulkTeamMemberBudgetUpdateRequest.model_validate(
                {"members": row * updates}
            )
        ),
    }


def scenario_team_non_object():
    module = load("management_endpoints.team_endpoints")
    return {
        "delete_string": attempts(
            lambda: module.BulkTeamMemberDeleteRequest.model_validate({"members": ["u"]})
        ),
        "delete_null": attempts(
            lambda: module.BulkTeamMemberDeleteRequest.model_validate({"members": [None]})
        ),
        "update_string": attempts(
            lambda: module.BulkTeamMemberBudgetUpdateRequest.model_validate({"members": ["u"]})
        ),
        "update_null": attempts(
            lambda: module.BulkTeamMemberBudgetUpdateRequest.model_validate({"members": [None]})
        ),
    }


def scenario_team_deviations():
    module = load("management_endpoints.team_endpoints")
    metadata = module.TeamListItem.model_validate(
        {"team_id": "t", "members_with_roles": [{"user_id": "u", "role": "user"}]}
    )
    return {
        "json_metadata": attempts(
            lambda: module.TeamListItem.model_validate({"team_id": "t", "metadata": '{"a": 1}'})
        ),
        "empty_roles": attempts(
            lambda: module.TeamListItem.model_validate({"team_id": "t", "members_with_roles": {}})
        ),
        "budget_duration": attempts(
            lambda: module.TeamMemberBudgetPatch.model_validate(
                {"user_id": "u", "budget_duration": "bogus"}
            )
        ),
        "bogus_route": attempts(
            lambda: module.BulkUpdateTeamMemberPermissionsRequest.model_validate(
                {"permissions": ["/bogus"]}
            )
        ),
        "member_row": metadata.model_dump(mode="json")["members_with_roles"],
    }


def scenario_sso_deviations():
    module = load("management_endpoints.ui_sso")
    return {
        "upperbound_name": module.LiteLLM_UpperboundKeyGenerateParams.__name__,
        "access_name": module.AccessControl_UI_AccessMode.__name__,
        "bogus_route": attempts(
            lambda: module.DefaultTeamSSOParams.model_validate(
                {"team_member_permissions": ["/bogus"]}
            )
        ),
        "bogus_role": attempts(
            lambda: module.RoleMappings.model_validate(
                {"provider": "p", "group_claim": "g", "default_role": "bogus"}
            )
        ),
        "bogus_role_key": attempts(
            lambda: module.RoleMappings.model_validate(
                {"provider": "p", "group_claim": "g", "roles": {"bogus": ["g"]}}
            )
        ),
    }


def scenario_public_presets():
    module = load("public_endpoints.public_endpoints")
    tiers = {"SIMPLE": ["a"], "MEDIUM": [], "COMPLEX": [], "REASONING": ["r"]}
    config = {"tiers": tiers, "future_field": {"x": 1}}
    record = {"label": "l", "description": "d", "complexity_router_config": config}
    scores = {
        "tier_boundaries": {"a": 1.5},
        "token_thresholds": {"b": 2},
        "dimension_weights": {"c": 0.5},
    }
    return {
        "tiers": dump(module.AutoRouterPresetTiers, tiers),
        "extra_tier": attempts(
            lambda: module.AutoRouterPresetTiers.model_validate(tiers | {"UNKNOWN": []})
        ),
        "config": dump(module.AutoRouterPresetConfig, config),
        "record": dump(module.AutoRouterPresetRecord, record),
        "scores": dump(module.ComplexityScorerDefaults, scores),
    }


SCENARIOS = {
    "scim_wire": scenario_scim_wire,
    "scim_overrides": scenario_scim_overrides,
    "scim_absent": scenario_scim_absent,
    "scim_deviations": scenario_scim_deviations,
    "scim_email": scenario_scim_email,
    "scim_json": scenario_scim_json,
    "scim_serialization": scenario_scim_serialization,
    "scim_user_dump": scenario_scim_user_dump,
    "scim_coercions": scenario_scim_coercions,
    "scim_patches": scenario_scim_patches,
    "team_valid": scenario_team_valid,
    "team_lists": scenario_team_lists,
    "team_rejections": scenario_team_rejections,
    "team_limits": scenario_team_limits,
    "team_non_object": scenario_team_non_object,
    "team_deviations": scenario_team_deviations,
    "sso_deviations": scenario_sso_deviations,
    "public_presets": scenario_public_presets,
}

spec = json.loads(sys.argv[2])
models = {}
described = {}
for module_name, names in spec["modules"].items():
    module = load(module_name)
    models[module_name] = {
        name: entry(getattr(module, name), name not in spec["validation_only"]) for name in names
    }
    described[module_name] = {
        name: descriptions(getattr(module, name))
        for name in names
        if not hasattr(getattr(module, name), "__required_keys__")
    }
constants = {}
for module_name, names in spec["constants"].items():
    module = load(module_name)
    constants[module_name] = {name: getattr(module, name) for name in names}
scenarios = {name: SCENARIOS[name]() for name in spec["scenarios"]}
report = {
    "models": models,
    "descriptions": described,
    "constants": constants,
    "scenarios": scenarios,
}
print(json.dumps(report, default=str))
"""

SCIM: Final = "management_endpoints.scim_v2"
TEAM: Final = "management_endpoints.team_endpoints"
SSO: Final = "management_endpoints.ui_sso"
UI: Final = "ui_sso"
PUBLIC: Final = "public_endpoints.public_endpoints"

SCIM_MODELS: Final[tuple[str, ...]] = (
    "LiteLLM_UserScimMetadata",
    "SCIMResource",
    "SCIMUserName",
    "SCIMUserEmail",
    "SCIMUserGroup",
    "SCIMMultiValuedAttribute",
    "SCIMUserManager",
    "SCIMEnterpriseUser",
    "SCIMUser",
    "SCIMMember",
    "SCIMGroup",
    "SCIMPlaceholderMergeResult",
    "SCIMListResponse",
    "SCIMPatchOperation",
    "SCIMPatchOp",
    "SCIMFeature",
    "SCIMServiceProviderConfig",
    "SCIMSchemaExtension",
    "SCIMResourceType",
    "SCIMSchemaAttribute",
    "SCIMSchema",
)

TEAM_MODELS: Final[tuple[str, ...]] = (
    "GetTeamMemberPermissionsRequest",
    "GetTeamMemberPermissionsResponse",
    "UpdateTeamMemberPermissionsRequest",
    "BulkUpdateTeamMemberPermissionsRequest",
    "BulkUpdateTeamMemberPermissionsResponse",
    "TeamListItem",
    "TeamListResponse",
    "BulkTeamMemberAddRequest",
    "TeamMemberAddResult",
    "BulkTeamMemberAddResponse",
    "TeamMemberRef",
    "BulkTeamMemberDeleteRequest",
    "TeamMemberDeleteResult",
    "BulkTeamMemberDeleteResponse",
    "TeamMemberBudgetPatch",
    "BulkTeamMemberBudgetUpdateRequest",
    "TeamMemberBudgetUpdateResult",
    "BulkTeamMemberBudgetUpdateResponse",
    "TeamMemberInfoResponse",
    "TeamMetadataFieldSchema",
    "TeamMetadataSchemaResponse",
    "TeamUserSpendRow",
    "TeamUserSpendResponse",
)

AUTO_ROUTER_MODELS: Final[tuple[str, ...]] = (
    "AutoRouterPresetTiers",
    "AutoRouterPresetConfig",
    "AutoRouterPresetRecord",
    "ComplexityScorerDefaults",
)

SSO_MODELS: Final[tuple[str, ...]] = (
    "LiteLLM_UpperboundKeyGenerateParams",
    "AccessControl_UI_AccessMode",
    "RoleMappings",
    "TeamMappings",
    "SSOConfig",
    "DefaultTeamSSOParams",
)

SSO_GROUPS: Final[tuple[str, ...]] = (
    "MicrosoftGraphAPIUserGroupDirectoryObject",
    "MicrosoftGraphAPIUserGroupResponse",
    "MicrosoftServicePrincipalTeam",
)

UI_GROUPS: Final[tuple[str, ...]] = ("ReturnedUITokenObject", "ParsedOpenIDResult")

PUBLIC_MODELS: Final[tuple[str, ...]] = (
    "PublicModelHubInfo",
    "ProviderCredentialField",
    "ProviderCreateInfo",
    "AgentCredentialField",
    "AgentCreateInfo",
    "EndpointProvider",
    "SupportedEndpoint",
    "SupportedEndpointsResponse",
    "AutoRouterPresetTiers",
    "AutoRouterPresetConfig",
    "AutoRouterPresetRecord",
    "ComplexityScorerDefaults",
)

MODULES: Final[dict[str, list[str]]] = {
    SCIM: list(SCIM_MODELS),
    TEAM: list(TEAM_MODELS),
    SSO: [*SSO_MODELS, *SSO_GROUPS],
    UI: list(UI_GROUPS),
    PUBLIC: list(PUBLIC_MODELS),
}

# LiteLLM's SCIMUser model_serializer erases its properties, so only validation mode is compared
VALIDATION_ONLY: Final[tuple[str, ...]] = ("SCIMUser",)

CONSTANTS: Final[dict[str, list[str]]] = {
    SCIM: [
        "SCIM_ENTERPRISE_USER_SCHEMA",
        "SCIM_ENTERPRISE_METADATA_KEY",
        "SCIM_ENTITLEMENTS_METADATA_KEY",
        "SCIM_ROLES_METADATA_KEY",
        "SCIM_MANAGED_TEAM_METADATA_KEY",
        "SCIM_TEAM_DATA_METADATA_KEY",
        "SCIM_MULTI_VALUED_ATTRIBUTE_METADATA_KEYS",
    ],
    TEAM: [
        "TeamIdSearchMatch",
        "MAX_BULK_TEAM_MEMBER_DELETES",
        "MAX_BULK_TEAM_MEMBER_BUDGET_UPDATES",
    ],
}

SCENARIOS: Final[tuple[str, ...]] = (
    "scim_wire",
    "scim_overrides",
    "scim_absent",
    "scim_deviations",
    "scim_email",
    "scim_json",
    "scim_serialization",
    "scim_user_dump",
    "scim_coercions",
    "scim_patches",
    "team_valid",
    "team_lists",
    "team_rejections",
    "team_limits",
    "team_non_object",
    "team_deviations",
    "sso_deviations",
    "public_presets",
)

SCIM_ENTERPRISE_USER_SCHEMA: Final = "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"


class ProbeSpec(TypedDict):
    modules: dict[str, list[str]]
    validation_only: list[str]
    constants: dict[str, list[str]]
    scenarios: list[str]


PROBE_SPEC: Final[ProbeSpec] = {
    "modules": MODULES,
    "validation_only": list(VALIDATION_ONLY),
    "constants": CONSTANTS,
    "scenarios": list(SCENARIOS),
}

SPEC: Final = json.dumps(PROBE_SPEC)

OBJECT: Final = TypeAdapter(dict[str, JsonValue])
STRINGS: Final = TypeAdapter(list[str])
ROWS: Final = TypeAdapter(list[dict[str, JsonValue]])
ERRORS: Final = TypeAdapter(list[list[JsonValue]])


def run_probe(prefix: str, spec: str) -> str:
    command = [sys.executable, "-c", PROBE, prefix, spec]
    done = subprocess.run(command, capture_output=True, text=True, check=False)
    assert done.returncode == 0, done.stderr
    return done.stdout


@pytest.fixture(scope="module")
def probe() -> dict[str, dict[str, JsonValue]]:
    return {
        "ours": OBJECT.validate_json(run_probe(BETTERLITE, SPEC)),
        "theirs": OBJECT.validate_json(run_probe(LITELLM, SPEC)),
    }


def module_entries(
    probe: dict[str, dict[str, JsonValue]], prefix: str, module: str
) -> dict[str, JsonValue]:
    models = OBJECT.validate_python(probe[prefix]["models"])
    return OBJECT.validate_python(models[module])


def scenario_entries(
    probe: dict[str, dict[str, JsonValue]], prefix: str, name: str
) -> dict[str, JsonValue]:
    scenarios = OBJECT.validate_python(probe[prefix]["scenarios"])
    return OBJECT.validate_python(scenarios[name])


MATCHING_SCENARIOS: Final[tuple[str, ...]] = (
    "scim_wire",
    "scim_overrides",
    "scim_absent",
    "scim_email",
    "scim_user_dump",
    "scim_coercions",
    "scim_patches",
    "team_valid",
    "team_lists",
    "team_rejections",
    "team_limits",
    "public_presets",
)


@pytest.mark.parametrize("name", MATCHING_SCENARIOS)
def test_names_scenario_matches(
    probe: dict[str, dict[str, JsonValue]], name: str
) -> None:
    theirs = scenario_entries(probe, "theirs", name)
    assert scenario_entries(probe, "ours", name) == theirs


def test_names_scim_models(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = module_entries(probe, "ours", SCIM)
    reference = module_entries(probe, "theirs", SCIM)
    assert ours == reference


def test_names_team_models(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = module_entries(probe, "ours", TEAM)
    reference = module_entries(probe, "theirs", TEAM)
    assert ours == reference


def test_names_public_models(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = module_entries(probe, "ours", PUBLIC)
    reference = module_entries(probe, "theirs", PUBLIC)
    assert ours == reference


def test_names_ui_typed_dicts(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = module_entries(probe, "ours", UI)
    reference = module_entries(probe, "theirs", UI)
    assert ours == reference


def test_names_sso_models(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = module_entries(probe, "ours", SSO)
    reference = module_entries(probe, "theirs", SSO)
    assert {name: ours[name] for name in SSO_MODELS} == {
        name: reference[name] for name in SSO_MODELS
    }


def test_names_sso_typed_dicts(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = module_entries(probe, "ours", SSO)
    reference = module_entries(probe, "theirs", SSO)
    assert {name: ours[name] for name in SSO_GROUPS} == {
        name: reference[name] for name in SSO_GROUPS
    }


def test_names_descriptions(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = OBJECT.validate_python(probe["ours"]["descriptions"])
    reference = OBJECT.validate_python(probe["theirs"]["descriptions"])
    ours_sso = OBJECT.validate_python(ours[SSO])
    reference_sso = OBJECT.validate_python(reference[SSO])
    for name in SSO_MODELS:
        ours_entry = OBJECT.validate_python(ours_sso[name])
        reference_entry = OBJECT.validate_python(reference_sso[name])
        assert ours_entry == reference_entry, name
    ours_public = OBJECT.validate_python(ours[PUBLIC])
    reference_public = OBJECT.validate_python(reference[PUBLIC])
    for name in AUTO_ROUTER_MODELS:
        ours_entry = OBJECT.validate_python(ours_public[name])
        reference_entry = OBJECT.validate_python(reference_public[name])
        assert ours_entry["root"] is None, name
        assert reference_entry["root"] is not None, name


def test_names_scim_constants(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = OBJECT.validate_python(probe["ours"]["constants"])
    reference = OBJECT.validate_python(probe["theirs"]["constants"])
    assert ours[SCIM] == reference[SCIM]


def test_names_team_constants(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = OBJECT.validate_python(probe["ours"]["constants"])
    reference = OBJECT.validate_python(probe["theirs"]["constants"])
    assert ours[TEAM] == reference[TEAM]


def test_names_scim_dump_overrides(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = scenario_entries(probe, "ours", "scim_overrides")
    resource = OBJECT.validate_python(ours["resource_default"])
    extension = OBJECT.validate_python(ours["extension_default"])
    attribute = OBJECT.validate_python(ours["attribute_default"])
    assert "schemaExtensions" not in resource
    assert "schema" in extension
    assert "subAttributes" not in attribute


def test_names_scim_absent_blocks(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = scenario_entries(probe, "ours", "scim_absent")
    wire = STRINGS.validate_python(ours["wire"])
    assert not {"entitlements", "roles"} & set(wire)
    assert SCIM_ENTERPRISE_USER_SCHEMA not in wire


def test_names_scim_deviations(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = scenario_entries(probe, "ours", "scim_deviations")
    reference = scenario_entries(probe, "theirs", "scim_deviations")
    assert ours["metadata_name"] == "UserScimMetadata"
    assert reference["metadata_name"] == "LiteLLM_UserScimMetadata"
    assert OBJECT.validate_python(ours["bad_email"])["raised"] is None
    assert OBJECT.validate_python(reference["bad_email"])["raised"] == "ValidationError"
    ours_fields = STRINGS.validate_python(ours["fields"])
    reference_fields = STRINGS.validate_python(reference["fields"])
    assert "user_name" in ours_fields
    assert "userName" in reference_fields
    ours_snake = OBJECT.validate_python(ours["snake_case"])
    reference_snake = OBJECT.validate_python(reference["snake_case"])
    assert ours_snake["userName"] == "alice"
    assert reference_snake["userName"] is None
    assert ours["serialization_properties"] is True
    assert reference["serialization_properties"] is False


def test_names_scim_email(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = scenario_entries(probe, "ours", "scim_email")
    normalized = {
        case: OBJECT.validate_python(OBJECT.validate_python(ours[case])["json"])["value"]
        for case in ours
    }
    assert normalized == {
        "Alice@EXAMPLE.COM": "Alice@example.com",
        "  bob@x.io ": "bob@x.io",
    }


def test_names_scim_serialization(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = scenario_entries(probe, "ours", "scim_serialization")
    reference = scenario_entries(probe, "theirs", "scim_serialization")
    assert ours["has_properties"] is True
    assert reference["has_properties"] is False


def test_names_scim_json_fields(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = scenario_entries(probe, "ours", "scim_json")
    reference = scenario_entries(probe, "theirs", "scim_json")
    assert OBJECT.validate_python(ours["set_meta"])["raised"] == "ValidationError"
    assert OBJECT.validate_python(reference["set_meta"])["raised"] is None


def test_names_scim_user_dump(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = scenario_entries(probe, "ours", "scim_user_dump")
    user = OBJECT.validate_python(ours["user"])
    assert SCIM_ENTERPRISE_USER_SCHEMA in STRINGS.validate_python(user["wire"])
    roles = ROWS.validate_python(OBJECT.validate_python(user["json"])["roles"])
    assert roles[0]["value"] == "bare-role"


def test_names_scim_coercions(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = scenario_entries(probe, "ours", "scim_coercions")
    attribute = OBJECT.validate_python(ours["attribute"])
    member = OBJECT.validate_python(ours["member"])
    assert (attribute["value"], member["type"]) == ("bare", None)


def test_names_scim_patch_ops(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = scenario_entries(probe, "ours", "scim_patches")
    assert ours["normalized"] == "add"
    bogus = OBJECT.validate_python(ours["bogus"])
    assert bogus["raised"] == "ValidationError"


def test_names_team_rejections(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = scenario_entries(probe, "ours", "team_rejections")
    raised = [OBJECT.validate_python(ours[case])["raised"] for case in ours]
    assert raised == ["ValidationError"] * len(ours)


def test_names_team_member_limit(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = scenario_entries(probe, "ours", "team_limits")
    outcomes = {case: OBJECT.validate_python(ours[case])["raised"] for case in ours}
    assert outcomes["over_deletes"] == "ValidationError"
    assert outcomes["over_updates"] == "ValidationError"
    assert outcomes["at_deletes"] is None
    assert outcomes["at_updates"] is None


def test_names_team_non_object(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = scenario_entries(probe, "ours", "team_non_object")
    reference = scenario_entries(probe, "theirs", "team_non_object")
    assert list(ours) == list(reference)
    for case in ours:
        our_outcome = OBJECT.validate_python(ours[case])
        assert our_outcome["raised"] == "ValidationError", case
        errors = ERRORS.validate_python(our_outcome["errors"])
        assert errors[0] == ["model_type", ["members", 0]], case
        assert OBJECT.validate_python(reference[case])["raised"] == "AttributeError", case


def test_names_team_deviations(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = scenario_entries(probe, "ours", "team_deviations")
    reference = scenario_entries(probe, "theirs", "team_deviations")
    for case in ("json_metadata", "empty_roles"):
        assert OBJECT.validate_python(ours[case])["raised"] == "ValidationError", case
        assert OBJECT.validate_python(reference[case])["raised"] is None, case
    for case in ("budget_duration", "bogus_route"):
        assert OBJECT.validate_python(ours[case])["raised"] is None, case
        assert OBJECT.validate_python(reference[case])["raised"] == "ValidationError", case
    assert ROWS.validate_python(ours["member_row"]) == [{"user_id": "u", "role": "user"}]
    assert ROWS.validate_python(reference["member_row"]) == [
        {"user_id": "u", "user_email": None, "role": "user"}
    ]


def test_names_sso_deviations(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = scenario_entries(probe, "ours", "sso_deviations")
    reference = scenario_entries(probe, "theirs", "sso_deviations")
    assert ours["upperbound_name"] == "UpperboundKeyGenerateParams"
    assert reference["upperbound_name"] == "LiteLLM_UpperboundKeyGenerateParams"
    assert ours["access_name"] == "UIAccessMode"
    assert reference["access_name"] == "AccessControl_UI_AccessMode"
    for case in ("bogus_route", "bogus_role", "bogus_role_key"):
        assert OBJECT.validate_python(ours[case])["raised"] is None, case
        assert OBJECT.validate_python(reference[case])["raised"] == "ValidationError", case


def test_names_public_presets(probe: dict[str, dict[str, JsonValue]]) -> None:
    ours = scenario_entries(probe, "ours", "public_presets")
    assert OBJECT.validate_python(ours["extra_tier"])["raised"] == "ValidationError"
