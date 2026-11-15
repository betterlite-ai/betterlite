import inspect
import subprocess
import sys
from typing import Final, TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter, ValidationError

from betterlite.types.proxy.guardrails.guardrail_hooks import (
    agent_365,
    aim,
    base,
    block_code_execution,
    cato_networks,
    cisco_ai_defense,
)
from betterlite.types.proxy.guardrails.guardrail_hooks.azure import (
    azure_prompt_shield,
    azure_text_moderation,
)
from betterlite.types.proxy.guardrails.guardrail_hooks.azure import (
    base as azure_base,
)

MODULE_NAMES: Final = (
    "agent_365",
    "aim",
    "akto",
    "alice",
    "aporia_ai",
    "azure.base",
    "azure.azure_prompt_shield",
    "azure.azure_text_moderation",
    "base",
    "bedrock_guardrails",
    "block_code_execution",
    "cato_networks",
    "cisco_ai_defense",
)
# Recorded from the pinned LiteLLM files, one entry per module
CLASS_COUNTS: Final = {
    "agent_365": 1,
    "aim": 1,
    "akto": 1,
    "alice": 1,
    "aporia_ai": 1,
    "azure.base": 1,
    "azure.azure_prompt_shield": 4,
    "azure.azure_text_moderation": 6,
    "base": 2,
    "bedrock_guardrails": 32,
    "block_code_execution": 2,
    "cato_networks": 1,
    "cisco_ai_defense": 3,
}
ALIAS_NAMES: Final = {
    "bedrock_guardrails": ("BedrockGuardrailQualifier",),
    "block_code_execution": ("CodeBlockActionTaken",),
    "azure.azure_text_moderation": ("AzureHarmCategories",),
    "cisco_ai_defense": (
        "CISCO_AI_DEFENSE_RULE_NAMES",
        "CISCO_AI_DEFENSE_INSPECTION_TYPE",
    ),
}
CONSTANT_NAMES: Final = {
    "agent_365": (
        "AGENT_365_PROD_API_BASE",
        "AGENT_365_PROD_RESOURCE_APP_ID",
        "AGENT_365_SCOPE_NAME",
    ),
    "block_code_execution": ("BLOCKED_LANGUAGES_OPTIONS",),
    "azure.azure_text_moderation": ("AZURE_CONTENT_SAFETY_CATEGORIES",),
}
DUMP_COUNT: Final = 26
REJECTION_COUNT: Final = 14
UI_COUNT: Final = 10
ABSTRACT_COUNT: Final = 15
DESCRIBE_SCRIPT = """
import importlib
import inspect
import json
import os
from enum import Enum

os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"

from pydantic import BaseModel, TypeAdapter, ValidationError
from typing_extensions import is_typeddict

PACKAGES = {
    "litellm": "litellm.types.proxy.guardrails.guardrail_hooks",
    "betterlite": "betterlite.types.proxy.guardrails.guardrail_hooks",
}
MODULES = (
    "agent_365",
    "aim",
    "akto",
    "alice",
    "aporia_ai",
    "azure.base",
    "azure.azure_prompt_shield",
    "azure.azure_text_moderation",
    "base",
    "bedrock_guardrails",
    "block_code_execution",
    "cato_networks",
    "cisco_ai_defense",
)
ALIASES = {
    "bedrock_guardrails": ("BedrockGuardrailQualifier",),
    "block_code_execution": ("CodeBlockActionTaken",),
    "azure.azure_text_moderation": ("AzureHarmCategories",),
    "cisco_ai_defense": (
        "CISCO_AI_DEFENSE_RULE_NAMES",
        "CISCO_AI_DEFENSE_INSPECTION_TYPE",
    ),
}
CONSTANTS = {
    "agent_365": (
        "AGENT_365_PROD_API_BASE",
        "AGENT_365_PROD_RESOURCE_APP_ID",
        "AGENT_365_SCOPE_NAME",
    ),
    "block_code_execution": ("BLOCKED_LANGUAGES_OPTIONS",),
    "azure.azure_text_moderation": ("AZURE_CONTENT_SAFETY_CATEGORIES",),
}
SAMPLES = [
    ("agent_365.default", "agent_365", "Agent365GuardrailConfigModel", {}),
    (
        "agent_365.configured",
        "agent_365",
        "Agent365GuardrailConfigModel",
        {
            "tenant_id": "tenant",
            "client_id": "client",
            "client_secret": "secret",
            "unreachable_fallback": "fail_open",
        },
    ),
    ("aim.default", "aim", "AimGuardrailConfigModel", {}),
    (
        "aim.configured",
        "aim",
        "AimGuardrailConfigModel",
        {"api_key": "key", "api_base": "https://aim.example", "inspect_embeddings": True},
    ),
    ("akto.default", "akto", "AktoConfigModel", {}),
    (
        "akto.configured",
        "akto",
        "AktoConfigModel",
        {
            "akto_base_url": "http://localhost:9090",
            "akto_api_key": "key",
            "akto_account_id": "1000000",
            "akto_vxlan_id": "0",
            "unreachable_fallback": "fail_open",
            "guardrail_timeout": 5,
        },
    ),
    ("alice.default", "alice", "AliceGuardrailConfigModel", {}),
    (
        "alice.configured",
        "alice",
        "AliceGuardrailConfigModel",
        {"api_key": "key", "api_base": "https://alice.example"},
    ),
    ("aporia_ai.default", "aporia_ai", "AporiaGuardrailConfigModel", {}),
    (
        "aporia_ai.configured",
        "aporia_ai",
        "AporiaGuardrailConfigModel",
        {"api_key": "key", "api_base": "https://aporia.example"},
    ),
    ("azure.base.default", "azure.base", "AzureContentSafetyConfigModel", {}),
    (
        "azure.base.configured",
        "azure.base",
        "AzureContentSafetyConfigModel",
        {"api_key": "key", "api_base": "https://azure.example", "api_version": "2025-01-01"},
    ),
    (
        "azure.azure_prompt_shield.default",
        "azure.azure_prompt_shield",
        "AzurePromptShieldGuardrailConfigModel",
        {},
    ),
    (
        "azure.azure_prompt_shield.configured",
        "azure.azure_prompt_shield",
        "AzurePromptShieldGuardrailConfigModel",
        {"cost_tier": "free", "price_per_1000_text_records": 0.0},
    ),
    (
        "azure.azure_text_moderation.params.default",
        "azure.azure_text_moderation",
        "AzureTextModerationOptionalParams",
        {},
    ),
    (
        "azure.azure_text_moderation.params.configured",
        "azure.azure_text_moderation",
        "AzureTextModerationOptionalParams",
        {
            "severity_threshold": 2,
            "severity_threshold_by_category": {"Hate": 2, "Violence": 4},
            "categories": ["Hate", "Sexual"],
            "blocklistNames": ["block"],
            "haltOnBlocklistHit": True,
            "outputType": "EightSeverityLevels",
        },
    ),
    (
        "azure.azure_text_moderation.config.configured",
        "azure.azure_text_moderation",
        "AzureContentSafetyTextModerationConfigModel",
        {"optional_params": {"severity_threshold": 2}},
    ),
    (
        "block_code_execution.default",
        "block_code_execution",
        "BlockCodeExecutionGuardrailConfigModel",
        {},
    ),
    (
        "block_code_execution.configured",
        "block_code_execution",
        "BlockCodeExecutionGuardrailConfigModel",
        {
            "blocked_languages": ["python"],
            "action": "mask",
            "confidence_threshold": 0.7,
            "detect_execution_intent": False,
        },
    ),
    ("cato_networks.default", "cato_networks", "CatoNetworksGuardrailConfigModel", {}),
    (
        "cato_networks.configured",
        "cato_networks",
        "CatoNetworksGuardrailConfigModel",
        {"api_key": "key", "api_base": "https://cato.example", "inspect_embeddings": True},
    ),
    (
        "cisco_ai_defense.rule.default",
        "cisco_ai_defense",
        "CiscoAIDefenseRule",
        {"rule_name": "PII"},
    ),
    (
        "cisco_ai_defense.rule.configured",
        "cisco_ai_defense",
        "CiscoAIDefenseRule",
        {"rule_name": "PCI", "entity_types": ["Email Address"]},
    ),
    (
        "cisco_ai_defense.params.default",
        "cisco_ai_defense",
        "CiscoAIDefenseGuardrailConfigModelOptionalParams",
        {},
    ),
    (
        "cisco_ai_defense.params.configured",
        "cisco_ai_defense",
        "CiscoAIDefenseGuardrailConfigModelOptionalParams",
        {
            "inspection_type": "mcp",
            "inspect_path": "/api/v1/inspect/mcp",
            "enabled_rules": [{"rule_name": "Harassment"}],
            "integration_profile_id": "profile",
            "integration_profile_version": "1",
            "integration_tenant_id": "tenant",
            "integration_type": "type",
            "on_flagged_action": "monitor",
            "fallback_on_error": "allow",
            "timeout": 30.0,
        },
    ),
    (
        "cisco_ai_defense.config.configured",
        "cisco_ai_defense",
        "CiscoAIDefenseGuardrailConfigModel",
        {
            "api_key": "key",
            "api_base": "https://cisco.example",
            "optional_params": {"timeout": 20.0},
        },
    ),
]
REJECTIONS = [
    (
        "agent_365.fallback",
        "agent_365",
        "Agent365GuardrailConfigModel",
        {"unreachable_fallback": "open"},
    ),
    ("akto.fallback", "akto", "AktoConfigModel", {"unreachable_fallback": "block"}),
    (
        "block.action",
        "block_code_execution",
        "BlockCodeExecutionGuardrailConfigModel",
        {"action": "log_only"},
    ),
    (
        "block.confidence_high",
        "block_code_execution",
        "BlockCodeExecutionGuardrailConfigModel",
        {"confidence_threshold": 1.5},
    ),
    (
        "block.confidence_low",
        "block_code_execution",
        "BlockCodeExecutionGuardrailConfigModel",
        {"confidence_threshold": -0.1},
    ),
    ("cisco.rule_name", "cisco_ai_defense", "CiscoAIDefenseRule", {"rule_name": "Nope"}),
    (
        "cisco.inspection_type",
        "cisco_ai_defense",
        "CiscoAIDefenseGuardrailConfigModelOptionalParams",
        {"inspection_type": "http"},
    ),
    (
        "cisco.fallback_on_error",
        "cisco_ai_defense",
        "CiscoAIDefenseGuardrailConfigModelOptionalParams",
        {"fallback_on_error": "monitor"},
    ),
    (
        "cisco.timeout_low",
        "cisco_ai_defense",
        "CiscoAIDefenseGuardrailConfigModelOptionalParams",
        {"timeout": 0.5},
    ),
    (
        "cisco.timeout_high",
        "cisco_ai_defense",
        "CiscoAIDefenseGuardrailConfigModelOptionalParams",
        {"timeout": 61.0},
    ),
    (
        "azure.output_type",
        "azure.azure_text_moderation",
        "AzureTextModerationOptionalParams",
        {"outputType": "Nope"},
    ),
    (
        "azure.categories",
        "azure.azure_text_moderation",
        "AzureTextModerationOptionalParams",
        {"categories": ["Nope"]},
    ),
    (
        "azure.threshold_category",
        "azure.azure_text_moderation",
        "AzureTextModerationOptionalParams",
        {"severity_threshold_by_category": {"Nope": 1}},
    ),
    (
        "azure.optional_params_missing",
        "azure.azure_text_moderation",
        "AzureContentSafetyTextModerationConfigModel",
        {},
    ),
]


UI_EXTRA_KEYS = (
    "ui_type",
    "options",
    "min",
    "max",
    "step",
    "default_value",
    "examples",
)


def ui_extras(model):
    found = {}
    for field_name, field in model.model_fields.items():
        extras = field.json_schema_extra
        if not isinstance(extras, dict):
            continue
        picked = {}
        for key in UI_EXTRA_KEYS:
            if key in extras:
                item = extras[key]
                picked[key] = item.value if isinstance(item, Enum) else item
        if picked:
            found[field_name] = picked
    return found


def collect_literals(node, found):
    if isinstance(node, dict):
        enum_values = node.get("enum")
        if isinstance(enum_values, list):
            found.update(json.dumps(item, sort_keys=True) for item in enum_values)
        if "const" in node:
            found.add(json.dumps(node["const"], sort_keys=True))
        for value in node.values():
            collect_literals(value, found)
    elif isinstance(node, list):
        for item in node:
            collect_literals(item, found)


def type_shape(node):
    if not isinstance(node, dict) or not node:
        return "json"
    if "$ref" in node:
        return "json" if node["$ref"].endswith("/JsonValue") else {"$ref": node["$ref"]}
    if "anyOf" in node:
        return {"anyOf": [type_shape(item) for item in node["anyOf"]]}
    if "enum" in node:
        return {"enum": sorted(json.dumps(item, sort_keys=True) for item in node["enum"])}
    kind = node.get("type")
    if kind == "object":
        extra = node.get("additionalProperties")
        if extra is None or extra is True:
            return "json"
        inner = type_shape(extra)
        return "json" if inner == "json" else {"object": inner}
    if kind == "array":
        return {"array": type_shape(node.get("items", {}))}
    return kind if kind is not None else "json"


def schema_fields(schema):
    properties = schema.get("properties", {})
    literals = set()
    collect_literals(schema, literals)
    return {
        "keys": sorted(properties),
        "required": sorted(schema.get("required", [])),
        "defaults": {
            name: json.dumps(field["default"], sort_keys=True)
            for name, field in properties.items()
            if "default" in field
        },
        "literals": sorted(literals),
    }


def schema_types(schema):
    return {
        name: type_shape(field)
        for name, field in schema.get("properties", {}).items()
    }


def kind_of(value):
    if is_typeddict(value):
        return "typeddict"
    if isinstance(value, type) and issubclass(value, BaseModel):
        return "model"
    if isinstance(value, type) and issubclass(value, Enum):
        return "enum"
    if isinstance(value, type):
        return "class"
    return type(value).__name__


def describe(module_name, module):
    names = sorted(
        name
        for name, value in vars(module).items()
        if not name.startswith("_")
        and getattr(value, "__module__", None) == module.__name__
    )
    shapes = {}
    types = {}
    for name in names:
        value = vars(module)[name]
        if is_typeddict(value):
            schema = TypeAdapter(value).json_schema()
        elif isinstance(value, type) and issubclass(value, BaseModel):
            schema = value.model_json_schema()
        else:
            continue
        shapes[name] = schema_fields(schema)
        types[name] = schema_types(schema)
    aliases = {}
    for alias in ALIASES.get(module_name, ()):
        aliases[alias] = schema_fields(TypeAdapter(vars(module)[alias]).json_schema())
    extras = {}
    for name in names:
        value = vars(module)[name]
        if isinstance(value, type) and issubclass(value, BaseModel):
            extras[name] = ui_extras(value)
    return {
        "names": names,
        "kinds": {name: kind_of(vars(module)[name]) for name in names},
        "shapes": shapes,
        "types": types,
        "ui_extras": extras,
        "aliases": aliases,
        "constants": {
            constant: vars(module)[constant] for constant in CONSTANTS.get(module_name, ())
        },
    }


def model_value(package, module_name, model_name, kwargs):
    module = importlib.import_module(package + "." + module_name)
    return vars(module)[model_name](**kwargs)


def side_report(package_name):
    package = PACKAGES[package_name]
    modules = {}
    dumps = {}
    ui = {}
    abstract = {}
    rejections = {}
    for module_name in MODULES:
        module = importlib.import_module(package + "." + module_name)
        modules[module_name] = describe(module_name, module)
        for name in modules[module_name]["names"]:
            value = vars(module)[name]
            if not (isinstance(value, type) and issubclass(value, BaseModel)):
                continue
            label = module_name + "." + name
            abstract[label] = inspect.isabstract(value)
            if name != "GuardrailConfigModel" and hasattr(value, "ui_friendly_name"):
                ui[label] = value.ui_friendly_name()
    for label, module_name, model_name, kwargs in SAMPLES:
        instance = model_value(package, module_name, model_name, kwargs)
        dumps[label] = json.dumps(instance.model_dump(), sort_keys=True, default=str)
    for label, module_name, model_name, kwargs in REJECTIONS:
        try:
            model_value(package, module_name, model_name, kwargs)
        except ValidationError as error:
            rejections[label] = sorted(
                [item["type"], item["msg"], str(item["loc"])] for item in error.errors()
            )
        else:
            rejections[label] = []
    return {
        "modules": modules,
        "dumps": dumps,
        "ui": ui,
        "abstract": abstract,
        "rejections": rejections,
    }


print(json.dumps({
    "litellm": side_report("litellm"),
    "betterlite": side_report("betterlite"),
}))
"""


class Shape(TypedDict):
    keys: list[str]
    required: list[str]
    defaults: dict[str, str]
    literals: list[str]


class ModuleDescription(TypedDict):
    names: list[str]
    kinds: dict[str, str]
    shapes: dict[str, Shape]
    types: dict[str, dict[str, JsonValue]]
    ui_extras: dict[str, JsonValue]
    aliases: dict[str, Shape]
    constants: dict[str, JsonValue]


class SideReport(TypedDict):
    modules: dict[str, ModuleDescription]
    dumps: dict[str, str]
    ui: dict[str, str]
    abstract: dict[str, bool]
    rejections: dict[str, list[list[str]]]


class FullReport(TypedDict):
    litellm: SideReport
    betterlite: SideReport


@pytest.fixture(scope="module")
def report() -> FullReport:
    completed = subprocess.run(
        [sys.executable, "-c", DESCRIBE_SCRIPT],
        check=True,
        capture_output=True,
        text=True,
    )
    parsed: FullReport = TypeAdapter(FullReport).validate_json(completed.stdout)
    return parsed


def test_module_names(report: FullReport) -> None:
    assert tuple(report["litellm"]["modules"]) == MODULE_NAMES
    assert tuple(report["betterlite"]["modules"]) == MODULE_NAMES


@pytest.mark.parametrize("module_name", MODULE_NAMES)
def test_covered_names(report: FullReport, module_name: str) -> None:
    theirs = report["litellm"]["modules"][module_name]
    ours = report["betterlite"]["modules"][module_name]
    assert len(theirs["names"]) == CLASS_COUNTS[module_name], module_name
    assert ours["names"] == theirs["names"], module_name
    assert ours["kinds"] == theirs["kinds"], module_name
    assert ours["shapes"] == theirs["shapes"], module_name
    assert ours["ui_extras"] == theirs["ui_extras"], module_name
    assert ours["aliases"] == theirs["aliases"], module_name
    assert ours["constants"] == theirs["constants"], module_name


@pytest.mark.parametrize("module_name", MODULE_NAMES)
def test_member_names(report: FullReport, module_name: str) -> None:
    theirs = report["litellm"]["modules"][module_name]
    ours = report["betterlite"]["modules"][module_name]
    aliases: tuple[str, ...] = ()
    if module_name in ALIAS_NAMES:
        aliases = ALIAS_NAMES[module_name]
    constants: tuple[str, ...] = ()
    if module_name in CONSTANT_NAMES:
        constants = CONSTANT_NAMES[module_name]
    assert set(theirs["aliases"]) == set(aliases), module_name
    assert set(theirs["constants"]) == set(constants), module_name
    assert ours["aliases"] == theirs["aliases"], module_name
    assert ours["constants"] == theirs["constants"], module_name


def test_model_type_names(report: FullReport) -> None:
    for module_name in MODULE_NAMES:
        theirs = dict(report["litellm"]["modules"][module_name]["types"])
        ours = dict(report["betterlite"]["modules"][module_name]["types"])
        if module_name == "azure.azure_text_moderation":
            theirs.pop("AzureContentSafetyTextModerationConfigModel")
            ours.pop("AzureContentSafetyTextModerationConfigModel")
        assert ours == theirs, module_name


def test_text_moderation_names(report: FullReport) -> None:
    name = "AzureContentSafetyTextModerationConfigModel"
    theirs = report["litellm"]["modules"]["azure.azure_text_moderation"]
    ours = report["betterlite"]["modules"]["azure.azure_text_moderation"]
    assert ours["shapes"][name] == theirs["shapes"][name]
    assert ours["shapes"][name]["required"] == ["optional_params"]
    theirs_type = theirs["types"][name]["optional_params"]
    ours_type = ours["types"][name]["optional_params"]
    assert theirs_type == {"$ref": "#/$defs/AzureTextModerationOptionalParams"}
    assert ours_type == {"anyOf": [theirs_type, "null"]}


def test_dump_names(report: FullReport) -> None:
    theirs = report["litellm"]["dumps"]
    ours = report["betterlite"]["dumps"]
    assert len(theirs) == DUMP_COUNT
    assert ours == theirs


def test_rejection_names(report: FullReport) -> None:
    theirs = report["litellm"]["rejections"]
    ours = report["betterlite"]["rejections"]
    assert len(theirs) == REJECTION_COUNT
    assert ours == theirs
    assert ours["block.confidence_high"]
    assert ours["azure.optional_params_missing"]


def test_ui_names(report: FullReport) -> None:
    theirs = report["litellm"]["ui"]
    ours = report["betterlite"]["ui"]
    assert len(theirs) == UI_COUNT
    assert ours == theirs
    assert ours["aim.AimGuardrailConfigModel"] == "AIM Guardrail"
    assert ours["cisco_ai_defense.CiscoAIDefenseGuardrailConfigModel"] == "Cisco AI Defense"


def test_ui_extra_names(report: FullReport) -> None:
    modules = report["betterlite"]["modules"]
    assert modules["aim"]["ui_extras"] == {
        "AimGuardrailConfigModel": {"inspect_embeddings": {"ui_type": "bool"}},
    }
    assert modules["cato_networks"]["ui_extras"] == {
        "CatoNetworksGuardrailConfigModel": {"inspect_embeddings": {"ui_type": "bool"}},
    }
    assert modules["akto"]["ui_extras"] == {
        "AktoConfigModel": {
            "akto_base_url": {
                "examples": ["http://localhost:9090", "https://akto-ingestion.example.com"],
            },
        },
    }
    assert modules["block_code_execution"]["ui_extras"] == {
        "BlockCodeExecutionGuardrailConfigModel": {
            "blocked_languages": {
                "ui_type": "multiselect",
                "options": block_code_execution.BLOCKED_LANGUAGES_OPTIONS,
            },
            "confidence_threshold": {
                "ui_type": "percentage",
                "min": 0.0,
                "max": 1.0,
                "step": 0.1,
                "default_value": 0.5,
            },
        },
    }


def test_abstract_names(report: FullReport) -> None:
    theirs = report["litellm"]["abstract"]
    ours = report["betterlite"]["abstract"]
    assert len(theirs) == ABSTRACT_COUNT
    assert ours == theirs
    assert ours["base.GuardrailConfigModel"] is True
    assert ours["azure.base.AzureContentSafetyConfigModel"] is False


def test_ui_friendly_names() -> None:
    assert aim.AimGuardrailConfigModel.ui_friendly_name() == "AIM Guardrail"
    assert cato_networks.CatoNetworksGuardrailConfigModel.ui_friendly_name() == (
        "Cato Networks Guardrail"
    )
    assert cisco_ai_defense.CiscoAIDefenseGuardrailConfigModel.ui_friendly_name() == (
        "Cisco AI Defense"
    )
    assert azure_prompt_shield.AzurePromptShieldGuardrailConfigModel.ui_friendly_name() == (
        "Azure Content Safety Prompt Shield"
    )
    assert azure_text_moderation.AzureContentSafetyTextModerationConfigModel.ui_friendly_name() == (
        "Azure Content Safety Text Moderation"
    )


def test_description_names() -> None:
    assert aim.AimGuardrailConfigModel.model_fields["api_key"].description is None
    assert aim.AimGuardrailConfigModel.model_fields["api_base"].description is None


def test_constant_names() -> None:
    assert agent_365.AGENT_365_PROD_API_BASE == "https://agent365.svc.cloud.microsoft"
    assert agent_365.AGENT_365_PROD_RESOURCE_APP_ID == (
        "ea9ffc3e-8a23-4a7d-836d-234d7c7565c1"
    )
    assert agent_365.AGENT_365_SCOPE_NAME == "ThreatProtection.Evaluate.All"
    assert block_code_execution.BLOCKED_LANGUAGES_OPTIONS == [
        "python",
        "javascript",
        "typescript",
        "bash",
        "ruby",
        "go",
        "java",
        "csharp",
        "php",
        "c",
        "cpp",
        "rust",
        "sql",
    ]
    assert azure_text_moderation.AZURE_CONTENT_SAFETY_CATEGORIES == [
        "Hate",
        "SelfHarm",
        "Sexual",
        "Violence",
    ]


def test_alias_names() -> None:
    payload: dict[str, JsonValue] = {
        "blocklistNames": ["block"],
        "haltOnBlocklistHit": True,
        "outputType": "EightSeverityLevels",
    }
    params = azure_text_moderation.AzureTextModerationOptionalParams.model_validate(payload)
    assert params.blocklist_names == ["block"]
    assert params.halt_on_blocklist_hit is True
    assert params.output_type == "EightSeverityLevels"
    dumped = params.model_dump()
    assert dumped["blocklistNames"] == ["block"]
    assert dumped["haltOnBlocklistHit"] is True
    assert dumped["outputType"] == "EightSeverityLevels"


def test_required_names() -> None:
    field = azure_text_moderation.AzureContentSafetyTextModerationConfigModel.model_fields[
        "optional_params"
    ]
    assert field.is_required()
    missing: dict[str, JsonValue] = {}
    with pytest.raises(ValidationError):
        azure_text_moderation.AzureContentSafetyTextModerationConfigModel.model_validate(missing)


def test_inherited_names() -> None:
    config = azure_prompt_shield.AzurePromptShieldGuardrailConfigModel()
    assert config.api_version == "2024-09-01"
    assert config.cost_tier is None
    assert azure_base.AzureContentSafetyConfigModel().api_version == "2024-09-01"


def test_generic_base_names() -> None:
    assert inspect.isabstract(base.GuardrailConfigModel)
    assert list(base.GuardrailConfigModel.model_fields) == ["optional_params"]
    assert list(aim.AimGuardrailConfigModel.model_fields) == [
        "optional_params",
        "api_key",
        "api_base",
        "inspect_embeddings",
    ]


def test_optional_params_names() -> None:
    payload: dict[str, JsonValue] = {"optional_params": {"extra": 1}}
    config = aim.AimGuardrailConfigModel.model_validate(payload)
    assert config.model_dump()["optional_params"] == {}


def test_extra_field_names() -> None:
    payload: dict[str, JsonValue] = {"unknown_knob": True}
    config = aim.AimGuardrailConfigModel.model_validate(payload)
    assert config.model_dump()["unknown_knob"] is True


def test_rule_names() -> None:
    payload: dict[str, JsonValue] = {"rule_name": "PII"}
    rule = cisco_ai_defense.CiscoAIDefenseRule.model_validate(payload)
    assert rule.rule_name == "PII"
    assert rule.entity_types is None
