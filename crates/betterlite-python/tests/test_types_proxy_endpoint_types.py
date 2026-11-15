import json
import subprocess
import sys
from typing import Final

import pytest
from pydantic import JsonValue, TypeAdapter

LITELLM: Final = "litellm"
BETTERLITE: Final = "betterlite"

PROBE: Final = """
import dataclasses, importlib, inspect, json, sys, typing
from pydantic import BaseModel, TypeAdapter, ValidationError

root, mode, spec = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
BASE = root + ".types.proxy."
STAMP = "2026-01-01T00:00:00Z"
TENANT = "00000000-0000-0000-0000-00000000000a"
CLIENT = "00000000-0000-0000-0000-00000000000b"
PRINCIPAL = "00000000-0000-0000-0000-00000000000c"
ENTRA = {"provider": "microsoft_entra", "tenant_id": TENANT, "client_id": CLIENT}
UI = {
    "server_root_path": "/",
    "proxy_base_url": None,
    "auto_redirect_to_sso": False,
    "admin_ui_disabled": False,
    "sso_configured": False,
}


def load(name):
    return importlib.import_module(BASE + name)


def clean(node):
    if isinstance(node, list):
        return [clean(item) for item in node]
    if not isinstance(node, dict):
        return node
    if node == {"$ref": "#/$defs/JsonValue"}:
        return True
    kept = {}
    for key, item in node.items():
        text = key in ("title", "description") and isinstance(item, str)
        if not text and key != "JsonValue":
            kept[key] = clean(item)
    if kept.get("$defs") == {}:
        del kept["$defs"]
    return kept


def schema(obj):
    return clean(TypeAdapter(obj).json_schema())


def without_defaults(node):
    for item in [node, *node.get("$defs", {}).values()]:
        for prop in item.get("properties", {}).values():
            prop.pop("default", None)
    return node


def field_defaults(obj):
    defaults = {}
    for name, info in obj.model_fields.items():
        if info.is_required():
            defaults[name] = "required"
        elif info.default_factory is not None:
            defaults[name] = {"value": info.default_factory()}
        else:
            defaults[name] = {"value": info.default}
    return json.loads(json.dumps(defaults, default=str))


def describe(obj):
    if inspect.isclass(obj) and issubclass(obj, BaseModel):
        return {
            "kind": "model",
            "bases": [base.__name__ for base in obj.__bases__],
            "config": json.loads(json.dumps(dict(obj.model_config), default=str)),
            "defaults": field_defaults(obj),
            "schema": without_defaults(clean(obj.model_json_schema())),
        }
    if inspect.isclass(obj) and dataclasses.is_dataclass(obj):
        return {
            "kind": "dataclass",
            "frozen": obj.__dataclass_params__.frozen,
            "slots": hasattr(obj, "__slots__"),
            "fields": [[item.name, item.type] for item in dataclasses.fields(obj)],
        }
    if hasattr(obj, "__required_keys__"):
        return {
            "kind": "typeddict",
            "required": sorted(obj.__required_keys__),
            "optional": sorted(obj.__optional_keys__),
            "schema": schema(obj),
        }
    if inspect.isclass(obj):
        own = sorted(key for key, item in vars(obj).items() if callable(item))
        return {"kind": "class", "bases": [base.__name__ for base in obj.__bases__], "own": own}
    if typing.get_origin(obj) is not None:
        return {"kind": "alias", "schema": schema(obj)}
    return {"kind": "value", "value": obj}


def attempt(call):
    try:
        return {"ok": call()}
    except ValidationError as error:
        return {"error": str(error)}
    except (AttributeError, TypeError) as error:
        return {"raised": type(error).__name__}


def dump(value):
    return value.model_dump(mode="json")


def ids(value):
    keys = ("tenant_id", "client_id", "service_principal_id")
    return {key: dump(value)[key] for key in keys}


def scenario_defaults():
    claude = load("claude_code_endpoints")
    logs = load("callback_logs_endpoints")
    identity = load("agent_identity")
    ui = load("discovery_endpoints.ui_discovery_endpoints")
    market = claude.MarketplaceResponse(name="market", owner=claude.PluginOwner(name="owner"))
    report = logs.CallbackLogsResponse(processed=0, failed=0)
    endpoints = ui.UiDiscoveryEndpoints.model_validate(UI)
    return {
        "marketplace_plugins": market.plugins,
        "callback_failures": report.failures,
        "entra_dump": dump(identity.EntraIdentityConfig.model_validate(ENTRA)),
        "ui_dump": endpoints.model_dump_json(),
        "ui_workers": endpoints.workers,
    }


def scenario_redacted():
    cls = load("litellm_pre_call_utils").RedactedDict
    headers = cls({"Authorization": "Bearer secret"})
    clone = headers.copy()
    clone["extra"] = 1
    return {
        "repr": repr(headers),
        "str": str(headers),
        "content": dict(headers),
        "copy_repr": repr(clone),
        "copy_type": type(clone).__name__,
        "copy_content": dict(clone),
        "original_after_copy_edit": dict(headers),
        "is_dict": issubclass(cls, dict),
    }


def scenario_user_not_found():
    cls = load("auth.auth_checks").UserNotFoundError
    error = cls("user-1")
    return {
        "text": str(error),
        "args": list(error.args),
        "is_value_error": issubclass(cls, ValueError),
    }


def scenario_budget():
    module = load("carried_budget_state")
    samples = {
        "key": (
            module.KeyBudgetSnapshot,
            {"user_api_key_budget_reset_at": STAMP, "unrelated": "ignored"},
            {"user_api_key_budget_reset_at": "not-a-date"},
            {"budget_reset_at": STAMP},
            "budget_reset_at",
        ),
        "team": (
            module.TeamBudgetSnapshot,
            {
                "user_api_key_team_budget_reset_at": STAMP,
                "user_api_key_team_table_max_budget": 5.5,
            },
            {"user_api_key_team_budget_reset_at": "not-a-date"},
            {"budget_reset_at": STAMP, "max_budget": 5.5},
            "max_budget",
        ),
        "user": (
            module.UserBudgetSnapshot,
            {
                "user_api_key_user_budget_reset_at": STAMP,
                "user_api_key_user_table_max_budget": 7.5,
                "user_api_key_user_alias": "alias",
            },
            {"user_api_key_user_table_max_budget": "not-a-number"},
            {"budget_reset_at": STAMP, "max_budget": 7.5, "user_alias": "alias"},
            "user_alias",
        ),
        "org": (
            module.OrgBudgetSnapshot,
            {"user_api_key_org_spend": 1.5, "user_api_key_org_max_budget": None},
            {"user_api_key_org_spend": "not-a-number"},
            {"spend": 1.5, "max_budget": None},
            "spend",
        ),
    }
    result = {}
    for name, (cls, metadata, invalid, by_field, field) in samples.items():
        snapshot = cls.from_metadata(metadata)
        result[name + "_entries"] = dict(snapshot.metadata_entries())
        result[name + "_invalid"] = cls.from_metadata(invalid) is None
        result[name + "_empty"] = cls.from_metadata({}) is None
        result[name + "_by_field"] = dump(cls.model_validate(by_field))
        result[name + "_frozen"] = attempt(lambda: setattr(snapshot, field, None))
    return result


def scenario_entra():
    cls = load("agent_identity").EntraIdentityConfig
    config = cls.model_validate(ENTRA)

    def case(**changes):
        return attempt(lambda: ids(cls.model_validate({**ENTRA, **changes})))

    return {
        "base": case(),
        "issuer": config.issuer,
        "principal_braces": case(service_principal_id="{" + PRINCIPAL.upper() + "}"),
        "principal_none": case(service_principal_id=None),
        "principal_invalid": case(service_principal_id="nope"),
        "tenant_upper": case(tenant_id=TENANT.upper()),
        "client_upper": case(client_id=CLIENT.upper()),
        "tenant_invalid": case(tenant_id="nope"),
        "extra": case(unexpected=True),
        "frozen": attempt(lambda: setattr(config, "tenant_id", "changed")),
    }


def scenario_validators():
    spend = load("spend_capture_rate")
    plane = load("control_plane_endpoints")
    vantage = load("vantage_endpoints")
    limit = spend.SPEND_CAPTURE_RATE_MAX_RANGE_DAYS

    def build(cls, **fields):
        return attempt(lambda: dump(cls(**fields)))

    settings = spend.SpendCaptureRateCheckSettings
    worker = plane.WorkerRegistryEntry
    init = vantage.VantageInitRequest
    update = vantage.VantageSettingsUpdate
    return {
        "spend_default": build(settings),
        "spend_zero_days": build(settings, lookback_days=0),
        "spend_one_day": build(settings, lookback_days=1),
        "spend_max_days": build(settings, lookback_days=limit),
        "spend_over_max_days": build(settings, lookback_days=limit + 1),
        "threshold_zero": build(settings, threshold=0),
        "threshold_one": build(settings, threshold=1.0),
        "threshold_over": build(settings, threshold=1.01),
        "providers_empty": build(settings, providers=()),
        "spend_extra": build(settings, unexpected=1),
        "worker_ftp": build(worker, worker_id="w", name="n", url="ftp://worker"),
        "worker_no_scheme": build(worker, worker_id="w", name="n", url="worker"),
        "worker_http": build(worker, worker_id="w", name="n", url="http://worker"),
        "worker_https": build(worker, worker_id="w", name="n", url="https://worker"),
        "init_valid": build(init, api_key="key", integration_token="token"),
        "init_blank_key": build(init, api_key=" ", integration_token="token"),
        "init_empty_key": build(init, api_key="", integration_token="token"),
        "init_blank_token": build(init, api_key="key", integration_token=" "),
        "init_missing": build(init),
        "update_none": build(update, api_key=None),
        "update_blank_key": build(update, api_key=" "),
        "update_empty_token": build(update, integration_token=""),
        "update_key": build(update, api_key="key"),
        "update_token": build(update, integration_token="token"),
    }


def scenario_gateway():
    module = load("gateway_requests")
    make_key = module.GatewayRequestKey
    key = make_key(date="2026-01-01", category="chat", route="/v1")
    same = make_key(date="2026-01-01", category="chat", route="/v1")
    other = make_key(date="2026-01-01", category="chat", route="/v2")
    counts = module.GatewayRequestCounts(successful_requests=1, failed_requests=2)
    return {
        "key": dataclasses.asdict(key),
        "equal": key == same,
        "not_equal": key != other,
        "dict_lookup": {key: 1}.get(same),
        "dict_miss": {key: 1}.get(other),
        "set_size": len({key, same, other}),
        "key_has_dict": hasattr(key, "__dict__"),
        "key_frozen": attempt(lambda: setattr(key, "route", "x")),
        "counts_unchanged": dataclasses.asdict(counts),
        "counts_success": dataclasses.asdict(counts.plus(succeeded=True)),
        "counts_failure": dataclasses.asdict(counts.plus(succeeded=False)),
        "counts_frozen": attempt(lambda: setattr(counts, "failed_requests", 0)),
        "activity_default": dump(module.GatewayRequestActivityResponse()),
    }


SCENARIOS = {
    "defaults": scenario_defaults,
    "redacted": scenario_redacted,
    "user_not_found": scenario_user_not_found,
    "budget": scenario_budget,
    "entra": scenario_entra,
    "validators": scenario_validators,
    "gateway": scenario_gateway,
}

if mode == "describe":
    result = {}
    for module_name, names in spec.items():
        module = load(module_name)
        result[module_name] = {
            name: describe(getattr(module, name)) if hasattr(module, name) else {"kind": "missing"}
            for name in names
        }
elif mode == "defined":
    result = {}
    for module_name in spec:
        module = load(module_name)
        result[module_name] = sorted(
            name
            for name, item in vars(module).items()
            if inspect.isclass(item) and item.__module__ == module.__name__
        )
else:
    result = {name: SCENARIOS[name]() for name in spec}
print(json.dumps(result, default=str))
"""

MODULES: Final[dict[str, list[str]]] = {
    "agent_identity": [
        "AgentExecutionMode",
        "EntraIdentityConfig",
        "AgentIdentityBinding",
        "AgentSubject",
        "AgentIdentityFailure",
        "ManagedAgentContext",
        "VerifiedHumanSubject",
        "MicrosoftInteractiveSubject",
        "ManagedAgentIdentityStatus",
    ],
    "auth.auth_checks": ["UserNotFoundError"],
    "callback_logs_endpoints": [
        "MAX_CALLBACK_LOG_RECORDS",
        "CallbackLogRecord",
        "CallbackLogsRequest",
        "CallbackLogFailure",
        "CallbackLogsResponse",
    ],
    "carried_budget_state": [
        "_BudgetSnapshot",
        "KeyBudgetSnapshot",
        "TeamBudgetSnapshot",
        "UserBudgetSnapshot",
        "OrgBudgetSnapshot",
    ],
    "claude_code_endpoints": [
        "PluginAuthor",
        "PluginOwner",
        "PluginSpec",
        "RegisterPluginRequest",
        "UpdatePluginRequest",
        "PluginResponse",
        "RegisterPluginResponse",
        "PluginListItem",
        "ListPluginsResponse",
        "MarketplacePluginEntry",
        "MarketplaceResponse",
    ],
    "cloudzero_endpoints": [
        "CloudZeroInitRequest",
        "CloudZeroInitResponse",
        "CloudZeroExportRequest",
        "CloudZeroExportResponse",
        "CloudZeroSettingsView",
        "CloudZeroSettingsUpdate",
    ],
    "compliance_endpoints": [
        "ComplianceCheckResult",
        "ComplianceResponse",
        "ComplianceCheckRequest",
    ],
    "control_plane_endpoints": ["WorkerRegistryEntry"],
    "discovery_endpoints.agent_skills_endpoints": [
        "AGENT_SKILLS_DISCOVERY_SCHEMA_URL",
        "MAX_SKILL_NAME_LENGTH",
        "MAX_SKILL_DESCRIPTION_LENGTH",
        "AgentSkillsIndexEntry",
        "AgentSkillsIndex",
    ],
    "discovery_endpoints.ui_discovery_endpoints": ["UiDiscoveryEndpoints"],
    "gateway_requests": [
        "GatewayRequestKey",
        "GatewayRequestCounts",
        "GatewayRequestSnapshot",
        "GatewayRequestBreakdownEntry",
        "GatewayRequestDailyEntry",
        "GatewayRequestActivityResponse",
    ],
    "litellm_pre_call_utils": ["RedactedDict", "SecretFields"],
    "model_access_group_budget": ["ModelAccessGroupBudget"],
    "model_deprecation": [
        "DEFAULT_DEPRECATION_WARN_DAYS",
        "DEFAULT_DEPRECATION_CHECK_INTERVAL_SECONDS",
        "DEPRECATION_IDLE_POLL_SECONDS",
        "DeprecationStatus",
        "ModelDeprecationInfo",
        "ModelDeprecationResponse",
    ],
    "model_listing": ["ModelInfoMetadata", "ModelInfoResponse"],
    "prompt_endpoints": ["TestPromptRequest"],
    "spend_capture_rate": [
        "SPEND_CAPTURE_RATE_MAX_RANGE_DAYS",
        "SpendCaptureProvider",
        "SpendCaptureRateCheckSettings",
        "CaptureRateDay",
        "CaptureRateReport",
    ],
    "vantage_endpoints": [
        "VantageInitRequest",
        "VantageInitResponse",
        "VantageExportRequest",
        "VantageDryRunRequest",
        "VantageExportResponse",
        "VantageSettingsView",
        "VantageSettingsUpdate",
    ],
}

SCENARIOS: Final = [
    "defaults",
    "redacted",
    "user_not_found",
    "budget",
    "entra",
    "validators",
    "gateway",
]

TENANT: Final = "00000000-0000-0000-0000-00000000000a"
CLIENT: Final = "00000000-0000-0000-0000-00000000000b"
PRINCIPAL: Final = "00000000-0000-0000-0000-00000000000c"
SECRET: Final[dict[str, JsonValue]] = {"Authorization": "Bearer secret"}
STAMP: Final = "2026-01-01T00:00:00Z"

FACTS: Final[list[tuple[str, dict[str, JsonValue]]]] = [
    ("defaults", {"marketplace_plugins": [], "callback_failures": [], "ui_workers": []}),
    (
        "redacted",
        {
            "repr": "RedactedDict(REDACTED)",
            "str": "RedactedDict(REDACTED)",
            "copy_repr": "RedactedDict(REDACTED)",
            "copy_type": "RedactedDict",
            "content": SECRET,
            "copy_content": {**SECRET, "extra": 1},
            "original_after_copy_edit": SECRET,
            "is_dict": True,
        },
    ),
    (
        "user_not_found",
        {
            "text": "User doesn't exist in db. 'user_id'=user-1. Create user via `/user/new` call.",
            "is_value_error": True,
        },
    ),
    (
        "entra",
        {
            "principal_braces": {
                "ok": {"tenant_id": TENANT, "client_id": CLIENT, "service_principal_id": PRINCIPAL}
            },
            "principal_none": {
                "ok": {"tenant_id": TENANT, "client_id": CLIENT, "service_principal_id": None}
            },
            "tenant_upper": {
                "ok": {"tenant_id": TENANT, "client_id": CLIENT, "service_principal_id": None}
            },
            "client_upper": {
                "ok": {"tenant_id": TENANT, "client_id": CLIENT, "service_principal_id": None}
            },
            "issuer": f"https://login.microsoftonline.com/{TENANT}/v2.0",
        },
    ),
    (
        "budget",
        {
            "key_entries": {"user_api_key_budget_reset_at": STAMP},
            "key_by_field": {"budget_reset_at": STAMP},
            "org_entries": {"user_api_key_org_spend": 1.5, "user_api_key_org_max_budget": None},
            "key_invalid": True,
            "team_invalid": True,
            "user_invalid": True,
            "org_invalid": True,
            "key_empty": True,
            "org_empty": True,
        },
    ),
    (
        "gateway",
        {
            "equal": True,
            "not_equal": True,
            "dict_lookup": 1,
            "dict_miss": None,
            "set_size": 2,
            "key_has_dict": False,
            "counts_unchanged": {"successful_requests": 1, "failed_requests": 2},
            "counts_success": {"successful_requests": 2, "failed_requests": 2},
            "counts_failure": {"successful_requests": 1, "failed_requests": 3},
        },
    ),
]

KINDS: Final[list[tuple[str, str, list[str]]]] = [
    ("budget", "error", ["key_frozen", "team_frozen", "user_frozen", "org_frozen"]),
    (
        "entra",
        "ok",
        ["base", "principal_braces", "principal_none", "tenant_upper", "client_upper"],
    ),
    ("entra", "error", ["principal_invalid", "tenant_invalid", "extra", "frozen"]),
    (
        "validators",
        "ok",
        [
            "spend_default",
            "spend_one_day",
            "spend_max_days",
            "threshold_one",
            "worker_http",
            "worker_https",
            "init_valid",
            "update_none",
            "update_key",
            "update_token",
        ],
    ),
    (
        "validators",
        "error",
        [
            "spend_zero_days",
            "spend_over_max_days",
            "threshold_zero",
            "threshold_over",
            "providers_empty",
            "spend_extra",
            "worker_ftp",
            "worker_no_scheme",
            "init_blank_key",
            "init_empty_key",
            "init_blank_token",
            "init_missing",
            "update_blank_key",
            "update_empty_token",
        ],
    ),
    ("gateway", "raised", ["key_frozen", "counts_frozen"]),
]

OBJECT: Final = TypeAdapter(dict[str, JsonValue])
STRINGS: Final = TypeAdapter(list[str])
OUTCOMES: Final = TypeAdapter(dict[str, dict[str, JsonValue]])


def run_probe(prefix: str, mode: str, spec: str) -> str:
    done = subprocess.run(
        [sys.executable, "-c", PROBE, prefix, mode, spec],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    return done.stdout


@pytest.fixture(scope="module")
def described() -> dict[str, dict[str, JsonValue]]:
    spec = json.dumps(MODULES)
    return {
        "theirs": OBJECT.validate_json(run_probe(LITELLM, "describe", spec)),
        "ours": OBJECT.validate_json(run_probe(BETTERLITE, "describe", spec)),
        "defined": OBJECT.validate_json(run_probe(LITELLM, "defined", spec)),
    }


@pytest.fixture(scope="module")
def behaved() -> dict[str, dict[str, dict[str, JsonValue]]]:
    spec = json.dumps(SCENARIOS)
    return {
        "theirs": OUTCOMES.validate_json(run_probe(LITELLM, "behave", spec)),
        "ours": OUTCOMES.validate_json(run_probe(BETTERLITE, "behave", spec)),
    }


@pytest.mark.parametrize("module", MODULES)
def test_names_defined(module: str, described: dict[str, dict[str, JsonValue]]) -> None:
    defined = STRINGS.validate_python(described["defined"][module])
    assert set(defined) <= set(MODULES[module])


@pytest.mark.parametrize("module", MODULES)
def test_names_match_litellm(module: str, described: dict[str, dict[str, JsonValue]]) -> None:
    theirs = OBJECT.validate_python(described["theirs"][module])
    ours = OBJECT.validate_python(described["ours"][module])
    assert all(item != {"kind": "missing"} for item in theirs.values())
    assert ours == theirs


@pytest.mark.parametrize("scenario", SCENARIOS)
def test_names_behaviour_match(
    scenario: str, behaved: dict[str, dict[str, dict[str, JsonValue]]]
) -> None:
    theirs = behaved["theirs"][scenario]
    assert len(theirs) > 1
    assert behaved["ours"][scenario] == theirs


@pytest.mark.parametrize(("scenario", "expected"), FACTS)
def test_names_facts(
    scenario: str,
    expected: dict[str, JsonValue],
    behaved: dict[str, dict[str, dict[str, JsonValue]]],
) -> None:
    ours = behaved["ours"][scenario]
    assert {case: ours[case] for case in expected} == expected


@pytest.mark.parametrize(("scenario", "kind", "cases"), KINDS)
def test_names_outcome_kinds(
    scenario: str,
    kind: str,
    cases: list[str],
    behaved: dict[str, dict[str, dict[str, JsonValue]]],
) -> None:
    for case in cases:
        outcome = OBJECT.validate_python(behaved["ours"][scenario][case])
        assert list(outcome) == [kind], case
