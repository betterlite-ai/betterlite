import logging
import subprocess
import sys
from typing import Final, TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter, ValidationError

from betterlite.types import guardrail_base_init
from betterlite.types.proxy.guardrails.guardrail_hooks import (
    tool_permission,
    typesafe,
    vigil_guard,
    xecguard,
    zscaler_ai_guard,
)

MODULE_NAMES: Final = (
    "proxy.guardrails.guardrail_hooks.tool_permission",
    "proxy.guardrails.guardrail_hooks.typesafe",
    "proxy.guardrails.guardrail_hooks.vigil_guard",
    "proxy.guardrails.guardrail_hooks.xecguard",
    "proxy.guardrails.guardrail_hooks.zscaler_ai_guard",
    "guardrail_base_init",
)
# Recorded from the pinned LiteLLM files, one entry per module
CLASS_COUNTS: Final = {
    "proxy.guardrails.guardrail_hooks.tool_permission": 4,
    "proxy.guardrails.guardrail_hooks.typesafe": 2,
    "proxy.guardrails.guardrail_hooks.vigil_guard": 1,
    "proxy.guardrails.guardrail_hooks.xecguard": 1,
    "proxy.guardrails.guardrail_hooks.zscaler_ai_guard": 1,
    "guardrail_base_init": 1,
}
DUMP_COUNT: Final = 20
NORMALIZATION_COUNT: Final = 5
REJECTION_COUNT: Final = 15
WARNING_COUNT: Final = 10
UI_COUNT: Final = 5
ABSTRACT_COUNT: Final = 9
DESCRIBE_SCRIPT = """
import importlib
import inspect
import json
import logging
import os
from enum import Enum

os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"

from pydantic import BaseModel, TypeAdapter, ValidationError
from typing_extensions import is_typeddict

PACKAGES = {
    "litellm": "litellm.types",
    "betterlite": "betterlite.types",
}
MODULES = (
    "proxy.guardrails.guardrail_hooks.tool_permission",
    "proxy.guardrails.guardrail_hooks.typesafe",
    "proxy.guardrails.guardrail_hooks.vigil_guard",
    "proxy.guardrails.guardrail_hooks.xecguard",
    "proxy.guardrails.guardrail_hooks.zscaler_ai_guard",
    "guardrail_base_init",
)
CONSTANTS = {
    "proxy.guardrails.guardrail_hooks.xecguard": ("XECGUARD_DEFAULT_POLICY_OPTIONS",),
}
TOOL_PERMISSION = "proxy.guardrails.guardrail_hooks.tool_permission"
TYPESAFE = "proxy.guardrails.guardrail_hooks.typesafe"
VIGIL_GUARD = "proxy.guardrails.guardrail_hooks.vigil_guard"
XECGUARD = "proxy.guardrails.guardrail_hooks.xecguard"
ZSCALER = "proxy.guardrails.guardrail_hooks.zscaler_ai_guard"
SAMPLES = [
    (
        "tool_permission.rule.default",
        TOOL_PERMISSION,
        "ToolPermissionRule",
        {"id": "rule-1", "decision": "allow", "tool_name": "read_file"},
    ),
    (
        "tool_permission.rule.configured",
        TOOL_PERMISSION,
        "ToolPermissionRule",
        {
            "id": "rule-2",
            "tool_name": "^read_.*",
            "tool_type": "function",
            "decision": "deny",
            "allowed_param_patterns": {"path": "^/tmp/.*"},
        },
    ),
    (
        "tool_permission.result.default",
        TOOL_PERMISSION,
        "ToolResult",
        {"tool_use_id": "toolu_1", "content": "ok"},
    ),
    (
        "tool_permission.result.configured",
        TOOL_PERMISSION,
        "ToolResult",
        {"type": "custom", "tool_use_id": "toolu_2", "content": "no", "is_error": False},
    ),
    (
        "tool_permission.error.default",
        TOOL_PERMISSION,
        "PermissionError",
        {"tool_name": "rm", "rule_id": None, "message": "denied"},
    ),
    (
        "tool_permission.error.configured",
        TOOL_PERMISSION,
        "PermissionError",
        {"tool_name": "rm", "rule_id": "rule-2", "message": "denied"},
    ),
    ("tool_permission.config.default", TOOL_PERMISSION, "ToolPermissionGuardrailConfigModel", {}),
    (
        "tool_permission.config.configured",
        TOOL_PERMISSION,
        "ToolPermissionGuardrailConfigModel",
        {
            "rules": [{"id": "r", "tool_name": "read", "decision": "allow"}],
            "default_action": "allow",
            "on_disallowed_action": "rewrite",
        },
    ),
    ("typesafe.params.default", TYPESAFE, "TypeSafeGuardrailOptionalParams", {}),
    (
        "typesafe.params.configured",
        TYPESAFE,
        "TypeSafeGuardrailOptionalParams",
        {"relevance_threshold": 0.5, "min_chars_to_evaluate": 10, "max_result_chars_in_state": 100},
    ),
    (
        "typesafe.params.bounds",
        TYPESAFE,
        "TypeSafeGuardrailOptionalParams",
        {"relevance_threshold": 0.0, "min_chars_to_evaluate": 0, "max_result_chars_in_state": 1},
    ),
    ("typesafe.config.default", TYPESAFE, "TypeSafeGuardrailConfigModel", {}),
    (
        "typesafe.config.configured",
        TYPESAFE,
        "TypeSafeGuardrailConfigModel",
        {
            "api_key": "key",
            "api_base": "https://typesafe.example",
            "model": "jev-latest",
            "unreachable_fallback": "fail_closed",
            "optional_params": {"relevance_threshold": 0.2},
        },
    ),
    ("vigil_guard.config.default", VIGIL_GUARD, "VigilGuardGuardrailConfigModel", {}),
    (
        "vigil_guard.config.configured",
        VIGIL_GUARD,
        "VigilGuardGuardrailConfigModel",
        {"api_base": "https://vigil.example", "api_key": "key"},
    ),
    ("xecguard.config.default", XECGUARD, "XecGuardConfigModel", {}),
    (
        "xecguard.config.configured",
        XECGUARD,
        "XecGuardConfigModel",
        {
            "api_key": "xgs_key",
            "api_base": "https://xec.example",
            "xecguard_model": "xecguard_v2",
            "policy_names": ["Default_Policy_SkillsProtection"],
            "block_on_error": True,
            "grounding_strictness": "STRICT",
        },
    ),
    ("zscaler.config.default", ZSCALER, "ZscalerAIGuardConfigModel", {}),
    (
        "zscaler.config.execute",
        ZSCALER,
        "ZscalerAIGuardConfigModel",
        {
            "api_key": "key",
            "api_base": "https://gw/v1/detection/execute-policy",
            "policy_id": 7,
            "timeout": 5.0,
        },
    ),
    (
        "zscaler.config.resolve",
        ZSCALER,
        "ZscalerAIGuardConfigModel",
        {
            "api_base": "https://gw/v1/detection/resolve-and-execute-policy",
            "send_user_api_key_alias": True,
            "send_user_api_key_user_id": True,
            "send_user_api_key_team_id": True,
        },
    ),
]
NORMALIZATIONS = [
    (
        "tool_permission.rule.upper_decision",
        TOOL_PERMISSION,
        "ToolPermissionRule",
        {"id": "r", "decision": "ALLOW", "tool_name": "read"},
    ),
    (
        "tool_permission.rule.blank_name",
        TOOL_PERMISSION,
        "ToolPermissionRule",
        {"id": "r", "decision": "allow", "tool_name": "   ", "tool_type": "function"},
    ),
    (
        "tool_permission.rule.padded_name",
        TOOL_PERMISSION,
        "ToolPermissionRule",
        {"id": "r", "decision": "allow", "tool_name": "  read_file  "},
    ),
    (
        "tool_permission.rule.blank_type",
        TOOL_PERMISSION,
        "ToolPermissionRule",
        {"id": "r", "decision": "allow", "tool_name": "read", "tool_type": "   "},
    ),
    (
        "tool_permission.config.upper_actions",
        TOOL_PERMISSION,
        "ToolPermissionGuardrailConfigModel",
        {"default_action": "ALLOW", "on_disallowed_action": "REWRITE"},
    ),
]
REJECTIONS = [
    (
        "tool_permission.rule.no_target",
        TOOL_PERMISSION,
        "ToolPermissionRule",
        {"id": "r", "decision": "allow"},
    ),
    (
        "tool_permission.rule.blank_type_only",
        TOOL_PERMISSION,
        "ToolPermissionRule",
        {"id": "r", "decision": "allow", "tool_type": "   "},
    ),
    (
        "tool_permission.rule.decision",
        TOOL_PERMISSION,
        "ToolPermissionRule",
        {"id": "r", "decision": "maybe", "tool_name": "x"},
    ),
    (
        "tool_permission.rule.id_missing",
        TOOL_PERMISSION,
        "ToolPermissionRule",
        {"decision": "allow", "tool_name": "x"},
    ),
    (
        "tool_permission.result.missing",
        TOOL_PERMISSION,
        "ToolResult",
        {"content": "x"},
    ),
    (
        "tool_permission.error.rule_id_missing",
        TOOL_PERMISSION,
        "PermissionError",
        {"tool_name": "x", "message": "m"},
    ),
    (
        "tool_permission.config.action",
        TOOL_PERMISSION,
        "ToolPermissionGuardrailConfigModel",
        {"default_action": "log"},
    ),
    (
        "typesafe.params.relevance_high",
        TYPESAFE,
        "TypeSafeGuardrailOptionalParams",
        {"relevance_threshold": 1.5},
    ),
    (
        "typesafe.params.relevance_low",
        TYPESAFE,
        "TypeSafeGuardrailOptionalParams",
        {"relevance_threshold": -0.1},
    ),
    (
        "typesafe.params.min_chars",
        TYPESAFE,
        "TypeSafeGuardrailOptionalParams",
        {"min_chars_to_evaluate": -1},
    ),
    (
        "typesafe.params.max_chars",
        TYPESAFE,
        "TypeSafeGuardrailOptionalParams",
        {"max_result_chars_in_state": 0},
    ),
    (
        "typesafe.config.fallback",
        TYPESAFE,
        "TypeSafeGuardrailConfigModel",
        {"unreachable_fallback": "open"},
    ),
    (
        "xecguard.config.grounding",
        XECGUARD,
        "XecGuardConfigModel",
        {"grounding_strictness": "LOOSE"},
    ),
    (
        "zscaler.config.policy_id",
        ZSCALER,
        "ZscalerAIGuardConfigModel",
        {"policy_id": "abc"},
    ),
    (
        "zscaler.config.timeout",
        ZSCALER,
        "ZscalerAIGuardConfigModel",
        {"timeout": "soon"},
    ),
]
WARNINGS = [
    (
        "zscaler.execute.no_policy",
        ZSCALER,
        "ZscalerAIGuardConfigModel",
        {"api_base": "https://gw/v1/detection/execute-policy"},
        {},
    ),
    (
        "zscaler.default_base",
        ZSCALER,
        "ZscalerAIGuardConfigModel",
        {},
        {},
    ),
    (
        "zscaler.execute.zero_policy",
        ZSCALER,
        "ZscalerAIGuardConfigModel",
        {"api_base": "https://gw/v1/detection/execute-policy", "policy_id": 0},
        {},
    ),
    (
        "zscaler.execute.min_policy",
        ZSCALER,
        "ZscalerAIGuardConfigModel",
        {"api_base": "https://gw/v1/detection/execute-policy", "policy_id": 1},
        {},
    ),
    (
        "zscaler.execute.policy",
        ZSCALER,
        "ZscalerAIGuardConfigModel",
        {"api_base": "https://gw/v1/detection/execute-policy", "policy_id": 4},
        {},
    ),
    (
        "zscaler.resolve.no_alias",
        ZSCALER,
        "ZscalerAIGuardConfigModel",
        {"api_base": "https://gw/v1/detection/resolve-and-execute-policy"},
        {},
    ),
    (
        "zscaler.resolve.alias",
        ZSCALER,
        "ZscalerAIGuardConfigModel",
        {
            "api_base": "https://gw/v1/detection/resolve-and-execute-policy",
            "send_user_api_key_alias": True,
        },
        {},
    ),
    (
        "zscaler.env.url",
        ZSCALER,
        "ZscalerAIGuardConfigModel",
        {},
        {"ZSCALER_AI_GUARD_URL": "https://gw/v1/detection/resolve-and-execute-policy"},
    ),
    (
        "zscaler.env.policy",
        ZSCALER,
        "ZscalerAIGuardConfigModel",
        {"api_base": "https://gw/v1/detection/execute-policy"},
        {"ZSCALER_AI_GUARD_POLICY_ID": "11"},
    ),
    (
        "zscaler.env.policy_invalid",
        ZSCALER,
        "ZscalerAIGuardConfigModel",
        {"api_base": "https://gw/v1/detection/execute-policy"},
        {"ZSCALER_AI_GUARD_POLICY_ID": "eleven"},
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
    "ui_hint",
    "condition",
)


def strip_log_handlers():
    for name in ("LiteLLM", "LiteLLM Proxy", "LiteLLM Router"):
        logging.getLogger(name).handlers.clear()


class Capture(logging.Handler):
    def __init__(self):
        super().__init__()
        self.messages = []

    def emit(self, record):
        self.messages.append(record.getMessage())


def capture_warnings(thunk):
    capture = Capture()
    loggers = [logging.getLogger(name) for name in ("LiteLLM", "LiteLLM Proxy")]
    for logger in loggers:
        logger.addHandler(capture)
    try:
        thunk()
    finally:
        for logger in loggers:
            logger.removeHandler(capture)
    return capture.messages


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
        "constants": {
            constant: vars(module)[constant] for constant in CONSTANTS.get(module_name, ())
        },
    }


def model_value(package, module_name, model_name, kwargs):
    module = importlib.import_module(package + "." + module_name)
    return vars(module)[model_name](**kwargs)


def construct_with_env(package, module_name, model_name, kwargs, env):
    saved = {}
    for key, value in env.items():
        saved[key] = os.environ.get(key)
        os.environ[key] = value
    try:
        return model_value(package, module_name, model_name, kwargs)
    finally:
        for key, value in saved.items():
            if value is None:
                del os.environ[key]
            else:
                os.environ[key] = value


def side_report(package_name):
    package = PACKAGES[package_name]
    modules = {}
    dumps = {}
    normalizations = {}
    ui = {}
    abstract = {}
    rejections = {}
    warnings = {}
    for module_name in MODULES:
        module = importlib.import_module(package + "." + module_name)
        strip_log_handlers()
        modules[module_name] = describe(module_name, module)
        for name in modules[module_name]["names"]:
            value = vars(module)[name]
            if not (isinstance(value, type) and issubclass(value, BaseModel)):
                continue
            label = module_name + "." + name
            abstract[label] = inspect.isabstract(value)
            if hasattr(value, "ui_friendly_name"):
                ui[label] = value.ui_friendly_name()
    for label, module_name, model_name, kwargs in SAMPLES:
        instance = model_value(package, module_name, model_name, kwargs)
        dumps[label] = json.dumps(instance.model_dump(), sort_keys=True, default=str)
    for label, module_name, model_name, kwargs in NORMALIZATIONS:
        instance = model_value(package, module_name, model_name, kwargs)
        normalizations[label] = json.dumps(instance.model_dump(), sort_keys=True, default=str)
    for label, module_name, model_name, kwargs in REJECTIONS:
        try:
            model_value(package, module_name, model_name, kwargs)
        except ValidationError as error:
            rejections[label] = sorted(
                [item["type"], item["msg"], str(item["loc"])] for item in error.errors()
            )
        else:
            rejections[label] = []
    for label, module_name, model_name, kwargs, env in WARNINGS:
        warnings[label] = capture_warnings(
            lambda: construct_with_env(package, module_name, model_name, kwargs, env)
        )
    return {
        "modules": modules,
        "dumps": dumps,
        "normalizations": normalizations,
        "ui": ui,
        "abstract": abstract,
        "rejections": rejections,
        "warnings": warnings,
    }


os.environ.pop("ZSCALER_AI_GUARD_URL", None)
os.environ.pop("ZSCALER_AI_GUARD_POLICY_ID", None)
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
    constants: dict[str, JsonValue]


class SideReport(TypedDict):
    modules: dict[str, ModuleDescription]
    dumps: dict[str, str]
    normalizations: dict[str, str]
    ui: dict[str, str]
    abstract: dict[str, bool]
    rejections: dict[str, list[list[str]]]
    warnings: dict[str, list[str]]


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
    assert ours["types"] == theirs["types"], module_name
    assert ours["ui_extras"] == theirs["ui_extras"], module_name


def test_constant_names(report: FullReport) -> None:
    theirs = report["litellm"]["modules"][
        "proxy.guardrails.guardrail_hooks.xecguard"
    ]["constants"]
    ours = report["betterlite"]["modules"][
        "proxy.guardrails.guardrail_hooks.xecguard"
    ]["constants"]
    assert ours == theirs
    expected: list[JsonValue] = [
        "Default_Policy_SystemPromptEnforcement",
        "Default_Policy_GeneralPromptAttackProtection",
        "Default_Policy_ContentBiasProtection",
        "Default_Policy_HarmfulContentProtection",
        "Default_Policy_SkillsProtection",
        "Default_Policy_PIISensitiveDataProtection",
    ]
    assert ours["XECGUARD_DEFAULT_POLICY_OPTIONS"] == expected


def test_dump_names(report: FullReport) -> None:
    theirs = report["litellm"]["dumps"]
    ours = report["betterlite"]["dumps"]
    assert len(theirs) == DUMP_COUNT
    assert ours == theirs


def test_normalization_names(report: FullReport) -> None:
    theirs = report["litellm"]["normalizations"]
    ours = report["betterlite"]["normalizations"]
    assert len(theirs) == NORMALIZATION_COUNT
    assert ours == theirs
    adapter: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
    upper: dict[str, JsonValue] = adapter.validate_json(
        ours["tool_permission.rule.upper_decision"],
    )
    assert upper["decision"] == "allow"
    blank: dict[str, JsonValue] = adapter.validate_json(ours["tool_permission.rule.blank_name"])
    assert blank["tool_name"] is None
    padded: dict[str, JsonValue] = adapter.validate_json(
        ours["tool_permission.rule.padded_name"],
    )
    assert padded["tool_name"] == "read_file"
    upper_actions: dict[str, JsonValue] = {
        "rules": None,
        "default_action": "allow",
        "on_disallowed_action": "rewrite",
        "optional_params": None,
    }
    normalized: dict[str, JsonValue] = adapter.validate_json(
        ours["tool_permission.config.upper_actions"],
    )
    assert normalized == upper_actions


def test_rejection_names(report: FullReport) -> None:
    theirs = report["litellm"]["rejections"]
    ours = report["betterlite"]["rejections"]
    assert len(theirs) == REJECTION_COUNT
    assert ours == theirs
    assert ours["tool_permission.rule.no_target"]
    assert ours["typesafe.params.relevance_high"]
    assert ours["zscaler.config.policy_id"]


def test_warning_names(report: FullReport) -> None:
    theirs = report["litellm"]["warnings"]
    ours = report["betterlite"]["warnings"]
    assert len(theirs) == WARNING_COUNT
    assert ours == theirs
    assert ours["zscaler.default_base"]
    assert ours["zscaler.execute.no_policy"]
    assert ours["zscaler.resolve.no_alias"]
    assert ours["zscaler.env.policy_invalid"][0].endswith(": eleven")
    assert ours["zscaler.execute.policy"] == []
    assert ours["zscaler.execute.min_policy"] == []
    assert ours["zscaler.resolve.alias"] == []
    assert ours["zscaler.env.policy"] == []


def test_ui_names(report: FullReport) -> None:
    expected: dict[str, str] = {
        "proxy.guardrails.guardrail_hooks.tool_permission.ToolPermissionGuardrailConfigModel": (
            "LiteLLM Tool Permission Guardrail"
        ),
        "proxy.guardrails.guardrail_hooks.typesafe.TypeSafeGuardrailConfigModel": (
            "TypeSafe (Jev) Compaction"
        ),
        "proxy.guardrails.guardrail_hooks.vigil_guard.VigilGuardGuardrailConfigModel": (
            "Vigil Guard"
        ),
        "proxy.guardrails.guardrail_hooks.xecguard.XecGuardConfigModel": "XecGuard",
        "proxy.guardrails.guardrail_hooks.zscaler_ai_guard.ZscalerAIGuardConfigModel": (
            "Zscaler AI Guard"
        ),
    }
    ours = report["betterlite"]["ui"]
    assert len(ours) == UI_COUNT
    assert ours == expected
    assert report["litellm"]["ui"] == expected


def test_abstract_names(report: FullReport) -> None:
    theirs = report["litellm"]["abstract"]
    ours = report["betterlite"]["abstract"]
    assert len(theirs) == ABSTRACT_COUNT
    assert ours == theirs
    assert not any(ours.values())


def test_ui_extra_names(report: FullReport) -> None:
    modules = report["betterlite"]["modules"]
    zscaler_extras: dict[str, JsonValue] = {
        "ZscalerAIGuardConfigModel": {
            "api_base": {
                "examples": [
                    "https://api.us1.zseclipse.net/v1/detection/execute-policy",
                    "https://api.us1.zseclipse.net/v1/detection/resolve-and-execute-policy",
                ],
            },
            "policy_id": {
                "ui_hint": "conditional_required",
                "condition": "Required when api_base ends with /execute-policy",
            },
            "send_user_api_key_alias": {
                "ui_type": "bool",
                "ui_hint": "recommended_when",
                "condition": "Recommended when api_base ends with /resolve-and-execute-policy",
            },
            "send_user_api_key_user_id": {"ui_type": "bool"},
            "send_user_api_key_team_id": {"ui_type": "bool"},
        },
    }
    assert modules["proxy.guardrails.guardrail_hooks.zscaler_ai_guard"]["ui_extras"] == (
        zscaler_extras
    )
    xecguard_extras: dict[str, JsonValue] = {
        "XecGuardConfigModel": {
            "policy_names": {
                "ui_type": "multiselect",
                "options": [
                    "Default_Policy_SystemPromptEnforcement",
                    "Default_Policy_GeneralPromptAttackProtection",
                    "Default_Policy_ContentBiasProtection",
                    "Default_Policy_HarmfulContentProtection",
                    "Default_Policy_SkillsProtection",
                    "Default_Policy_PIISensitiveDataProtection",
                ],
            },
        },
    }
    assert modules["proxy.guardrails.guardrail_hooks.xecguard"]["ui_extras"] == xecguard_extras
    typesafe_extras: dict[str, JsonValue] = {
        "TypeSafeGuardrailConfigModel": {},
        "TypeSafeGuardrailOptionalParams": {},
    }
    assert modules["proxy.guardrails.guardrail_hooks.typesafe"]["ui_extras"] == typesafe_extras


def test_ui_friendly_names() -> None:
    assert tool_permission.ToolPermissionGuardrailConfigModel.ui_friendly_name() == (
        "LiteLLM Tool Permission Guardrail"
    )
    assert typesafe.TypeSafeGuardrailConfigModel.ui_friendly_name() == "TypeSafe (Jev) Compaction"
    assert vigil_guard.VigilGuardGuardrailConfigModel.ui_friendly_name() == "Vigil Guard"
    assert xecguard.XecGuardConfigModel.ui_friendly_name() == "XecGuard"
    assert zscaler_ai_guard.ZscalerAIGuardConfigModel.ui_friendly_name() == "Zscaler AI Guard"


def test_permission_error_names() -> None:
    assert tool_permission.PermissionError.__name__ == "PermissionError"
    assert list(tool_permission.PermissionError.model_fields) == [
        "tool_name",
        "rule_id",
        "message",
    ]
    payload: dict[str, JsonValue] = {"tool_name": "rm", "rule_id": None, "message": "denied"}
    denied = tool_permission.PermissionError.model_validate(payload)
    assert denied.model_dump() == payload


def test_required_names() -> None:
    rule_fields = tool_permission.ToolPermissionRule.model_fields
    assert rule_fields["id"].is_required()
    assert rule_fields["decision"].is_required()
    assert rule_fields["tool_name"].is_required() is False
    error_fields = tool_permission.PermissionError.model_fields
    assert error_fields["rule_id"].is_required()
    error_payload: dict[str, JsonValue] = {"tool_name": "rm", "message": "m"}
    with pytest.raises(ValidationError):
        tool_permission.PermissionError.model_validate(error_payload)
    rule_payload: dict[str, JsonValue] = {"decision": "allow", "tool_name": "x"}
    with pytest.raises(ValidationError):
        tool_permission.ToolPermissionRule.model_validate(rule_payload)


def test_inherited_names() -> None:
    config = zscaler_ai_guard.ZscalerAIGuardConfigModel()
    assert config.api_key is None
    assert config.api_base is None
    assert config.policy_id is None
    assert config.send_user_api_key_alias is False
    assert config.send_user_api_key_user_id is False
    assert config.send_user_api_key_team_id is False
    assert config.timeout is None
    assert config.optional_params is None
    assert typesafe.TypeSafeGuardrailConfigModel().unreachable_fallback == "fail_open"
    assert tool_permission.ToolPermissionGuardrailConfigModel().default_action == "deny"
    assert tool_permission.ToolPermissionGuardrailConfigModel().on_disallowed_action == "block"


def test_guardrail_base_init_names() -> None:
    required_keys: frozenset[str] = frozenset()
    assert guardrail_base_init.GuardrailBaseInitKwargs.__required_keys__ == required_keys
    optional_keys: frozenset[str] = frozenset({
        "guardrail_name",
        "default_on",
        "mask_request_content",
        "mask_response_content",
        "violation_message_template",
        "end_session_after_n_fails",
        "on_violation",
        "realtime_violation_message",
        "on_sensitive_data",
        "sensitive_data_route_to_model",
        "sticky_session_routing",
        "run_in_parallel",
        "only_scan_new_messages",
    })
    assert guardrail_base_init.GuardrailBaseInitKwargs.__optional_keys__ == optional_keys


def test_warning_capture_names(
    caplog: pytest.LogCaptureFixture,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv("ZSCALER_AI_GUARD_URL", raising=False)
    monkeypatch.delenv("ZSCALER_AI_GUARD_POLICY_ID", raising=False)
    with caplog.at_level(logging.WARNING, logger="LiteLLM"):
        zscaler_ai_guard.ZscalerAIGuardConfigModel(
            api_base="https://gw/v1/detection/execute-policy",
        )
    assert "without a valid policy_id" in caplog.text
    caplog.clear()
    with caplog.at_level(logging.WARNING, logger="LiteLLM"):
        zscaler_ai_guard.ZscalerAIGuardConfigModel(
            api_base="https://gw/v1/detection/execute-policy",
            policy_id=3,
        )
    assert caplog.text == ""


def test_extra_field_names() -> None:
    payload: dict[str, JsonValue] = {"unknown_knob": True}
    config = tool_permission.ToolPermissionGuardrailConfigModel.model_validate(payload)
    assert config.model_dump()["unknown_knob"] is True


def test_optional_params_names() -> None:
    payload: dict[str, JsonValue] = {"optional_params": {"nested_knob": True}}
    config = tool_permission.ToolPermissionGuardrailConfigModel.model_validate(payload)
    assert config.optional_params is not None
    dumped = TypeAdapter(dict[str, JsonValue]).validate_json(config.model_dump_json())
    assert dumped["optional_params"] == {}


def test_description_absent_names() -> None:
    schema: dict[str, JsonValue] = tool_permission.ToolPermissionRule.model_json_schema()
    properties = TypeAdapter(dict[str, dict[str, JsonValue]]).validate_python(schema["properties"])
    assert all("description" not in field_schema for field_schema in properties.values())
