import subprocess
import sys
from typing import Final, TypedDict, TypeVar, get_args

import pytest
from pydantic import JsonValue, TypeAdapter, ValidationError

from betterlite.types.proxy.guardrails.guardrail_hooks import (
    hiddenlayer,
    javelin,
    lakera_ai_v2,
    litellm_content_filter,
    onyx,
)
from betterlite.types.proxy.guardrails.guardrail_hooks.ibm import ibm_detector
from betterlite.types.proxy.guardrails.guardrail_hooks.openai import openai_moderation

MODULE_NAMES: Final = (
    "hiddenlayer",
    "hide_secrets",
    "ibm.base",
    "ibm.ibm_detector",
    "javelin",
    "lakera_ai_v2",
    "lasso",
    "litellm_content_filter",
    "mcp_end_user_permission",
    "model_armor",
    "noma",
    "onyx",
    "openai.openai_moderation",
    "ovalix",
    "pangea",
)
# Recorded from the pinned LiteLLM files, one entry per module
CLASS_COUNTS: Final = {
    "hiddenlayer": 3,
    "hide_secrets": 1,
    "ibm.base": 1,
    "ibm.ibm_detector": 7,
    "javelin": 14,
    "lakera_ai_v2": 5,
    "lasso": 2,
    "litellm_content_filter": 9,
    "mcp_end_user_permission": 1,
    "model_armor": 1,
    "noma": 2,
    "onyx": 1,
    "openai.openai_moderation": 2,
    "ovalix": 1,
    "pangea": 2,
}
ALIAS_NAMES: Final = {
    "litellm_content_filter": (
        "CompetitorIntentType",
        "CompetitorActionHint",
        "ContentFilterDetection",
    ),
}
DUMP_COUNT: Final = 37
REJECTION_COUNT: Final = 23
UI_COUNT: Final = 15
ABSTRACT_COUNT: Final = 20
HIDDENLAYER_VERSION: Final = 2
STREAMING_SAMPLING_RATE: Final = 5
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
    "hiddenlayer",
    "hide_secrets",
    "ibm.base",
    "ibm.ibm_detector",
    "javelin",
    "lakera_ai_v2",
    "lasso",
    "litellm_content_filter",
    "mcp_end_user_permission",
    "model_armor",
    "noma",
    "onyx",
    "openai.openai_moderation",
    "ovalix",
    "pangea",
)
ALIASES = {
    "litellm_content_filter": (
        "CompetitorIntentType",
        "CompetitorActionHint",
        "ContentFilterDetection",
    ),
}
SAMPLES = [
    ("hiddenlayer.default", "hiddenlayer", "HiddenlayerGuardrailConfigModel", {}),
    (
        "hiddenlayer.configured",
        "hiddenlayer",
        "HiddenlayerGuardrailConfigModel",
        {
            "api_base": "https://hiddenlayer.example",
            "api_id": "client-id",
            "api_key": "secret",
            "version": 3,
        },
    ),
    ("hide_secrets.default", "hide_secrets", "HideSecretsGuardrailConfigModel", {}),
    (
        "hide_secrets.configured",
        "hide_secrets",
        "HideSecretsGuardrailConfigModel",
        {"detect_secrets_config": {"plugins_used": [{"name": "AWSKeyDetector"}]}},
    ),
    ("ibm.base.default", "ibm.base", "IBMGuardrailsBaseConfigModel", {}),
    (
        "ibm.base.configured",
        "ibm.base",
        "IBMGuardrailsBaseConfigModel",
        {
            "auth_token": "token",
            "base_url": "https://ibm.example",
            "detector_id": "jailbreak-detector",
            "is_detector_server": False,
            "verify_ssl": False,
        },
    ),
    ("ibm.params.default", "ibm.ibm_detector", "IBMDetectorOptionalParams", {}),
    (
        "ibm.params.configured",
        "ibm.ibm_detector",
        "IBMDetectorOptionalParams",
        {
            "detector_params": {"some": "param"},
            "extra_headers": {"x-extra": "1"},
            "score_threshold": 0.7,
            "block_on_detection": False,
        },
    ),
    ("ibm.config.default", "ibm.ibm_detector", "IBMDetectorGuardrailConfigModel", {}),
    (
        "ibm.config.configured",
        "ibm.ibm_detector",
        "IBMDetectorGuardrailConfigModel",
        {
            "auth_token": "token",
            "base_url": "https://ibm.example",
            "detector_id": "detector",
            "is_detector_server": True,
            "verify_ssl": True,
            "optional_params": {"score_threshold": 0.5},
        },
    ),
    ("javelin.default", "javelin", "JavelinGuardrailConfigModel", {}),
    (
        "javelin.configured",
        "javelin",
        "JavelinGuardrailConfigModel",
        {
            "guard_name": "guard",
            "api_version": "v2",
            "metadata": {"a": 1},
            "application": "application",
            "config": {"b": 2},
        },
    ),
    ("lasso.params.default", "lasso", "LassoGuardrailConfigModelOptionalParams", {}),
    (
        "lasso.params.configured",
        "lasso",
        "LassoGuardrailConfigModelOptionalParams",
        {"user_id": "user", "conversation_id": "conversation"},
    ),
    ("lasso.config.default", "lasso", "LassoGuardrailConfigModel", {}),
    (
        "lasso.config.configured",
        "lasso",
        "LassoGuardrailConfigModel",
        {
            "api_key": "key",
            "api_base": "https://lasso.example",
            "optional_params": {"user_id": "user"},
        },
    ),
    (
        "content_filter.category.default",
        "litellm_content_filter",
        "ContentFilterCategoryConfig",
        {"category": "harmful_self_harm", "action": "BLOCK"},
    ),
    (
        "content_filter.category.configured",
        "litellm_content_filter",
        "ContentFilterCategoryConfig",
        {
            "category": "harmful_bias",
            "enabled": False,
            "action": "MASK",
            "severity_threshold": "high",
            "category_file": "/path/to/custom_file.yaml",
        },
    ),
    (
        "content_filter.config.default",
        "litellm_content_filter",
        "LitellmContentFilterGuardrailConfigModel",
        {},
    ),
    (
        "content_filter.config.configured",
        "litellm_content_filter",
        "LitellmContentFilterGuardrailConfigModel",
        {
            "patterns": [{"pattern": "\\\\bsecret\\\\b"}],
            "blocked_words": [{"keyword": "blocked"}],
            "blocked_words_file": "blocked_words.yaml",
            "categories": [{"category": "harmful_self_harm", "action": "BLOCK"}],
            "severity_threshold": "high",
            "pattern_redaction_format": "[{pattern_name}_X]",
            "keyword_redaction_tag": "[X]",
            "competitor_intent_config": {"brand_self": ["ours"]},
        },
    ),
    (
        "mcp.default",
        "mcp_end_user_permission",
        "MCPEndUserPermissionGuardrailConfigModel",
        {},
    ),
    ("model_armor.default", "model_armor", "ModelArmorGuardrailConfigModel", {}),
    (
        "model_armor.configured",
        "model_armor",
        "ModelArmorGuardrailConfigModel",
        {
            "template_id": "template",
            "project_id": "project",
            "location": "us-central1",
            "credentials": "/credentials.json",
            "api_endpoint": "https://armor.example",
            "fail_on_error": False,
            "sanitize_error_detail": False,
        },
    ),
    ("noma.default", "noma", "NomaGuardrailConfigModel", {}),
    (
        "noma.configured",
        "noma",
        "NomaGuardrailConfigModel",
        {
            "use_v2": True,
            "api_key": "key",
            "api_base": "https://noma.example",
            "application_id": "application",
        },
    ),
    ("noma_v2.default", "noma", "NomaV2GuardrailConfigModel", {}),
    (
        "noma_v2.configured",
        "noma",
        "NomaV2GuardrailConfigModel",
        {
            "api_key": "key",
            "api_base": "https://noma.example",
            "application_id": "application",
            "gateway_name": "gateway",
            "monitor_mode": True,
            "block_failures": True,
        },
    ),
    ("onyx.default", "onyx", "OnyxGuardrailConfigModel", {}),
    (
        "onyx.configured",
        "onyx",
        "OnyxGuardrailConfigModel",
        {"api_base": "https://onyx.example", "api_key": "key", "timeout": 30.0},
    ),
    (
        "openai.default",
        "openai.openai_moderation",
        "OpenAIModerationGuardrailConfigModel",
        {},
    ),
    (
        "openai.configured",
        "openai.openai_moderation",
        "OpenAIModerationGuardrailConfigModel",
        {
            "model": "text-moderation-latest",
            "api_key": "key",
            "api_base": "https://openai.example/v1",
            "streaming_end_of_stream_only": True,
            "streaming_sampling_rate": 3,
        },
    ),
    ("ovalix.default", "ovalix", "OvalixGuardrailConfigModel", {}),
    (
        "ovalix.configured",
        "ovalix",
        "OvalixGuardrailConfigModel",
        {
            "tracker_api_base": "https://ovalix.example",
            "tracker_api_key": "key",
            "application_id": "application",
            "pre_checkpoint_id": "pre",
            "post_checkpoint_id": "post",
        },
    ),
    ("pangea.params.default", "pangea", "PangeaGuardrailConfigModelOptionalParams", {}),
    (
        "pangea.params.configured",
        "pangea",
        "PangeaGuardrailConfigModelOptionalParams",
        {"pangea_input_recipe": "input", "pangea_output_recipe": "output"},
    ),
    ("pangea.config.default", "pangea", "PangeaGuardrailConfigModel", {}),
    (
        "pangea.config.configured",
        "pangea",
        "PangeaGuardrailConfigModel",
        {
            "api_key": "key",
            "api_base": "https://pangea.example",
            "optional_params": {"pangea_input_recipe": "input"},
        },
    ),
]
REJECTIONS = [
    (
        "hiddenlayer.version",
        "hiddenlayer",
        "HiddenlayerGuardrailConfigModel",
        {"version": "two"},
    ),
    (
        "hide_secrets.config",
        "hide_secrets",
        "HideSecretsGuardrailConfigModel",
        {"detect_secrets_config": "nope"},
    ),
    (
        "ibm.base.flag",
        "ibm.base",
        "IBMGuardrailsBaseConfigModel",
        {"is_detector_server": "maybe"},
    ),
    (
        "ibm.params.threshold",
        "ibm.ibm_detector",
        "IBMDetectorOptionalParams",
        {"score_threshold": "high"},
    ),
    (
        "ibm.config.optional_params",
        "ibm.ibm_detector",
        "IBMDetectorGuardrailConfigModel",
        {"optional_params": "nested"},
    ),
    (
        "javelin.guard_name",
        "javelin",
        "JavelinGuardrailConfigModel",
        {"guard_name": 5},
    ),
    (
        "javelin.api_version",
        "javelin",
        "JavelinGuardrailConfigModel",
        {"api_version": 5},
    ),
    (
        "lasso.params.user_id",
        "lasso",
        "LassoGuardrailConfigModelOptionalParams",
        {"user_id": ["user"]},
    ),
    (
        "content_filter.action",
        "litellm_content_filter",
        "ContentFilterCategoryConfig",
        {"category": "category", "action": "ALLOW"},
    ),
    (
        "content_filter.severity_threshold",
        "litellm_content_filter",
        "ContentFilterCategoryConfig",
        {"category": "category", "action": "BLOCK", "severity_threshold": "critical"},
    ),
    (
        "content_filter.missing_category",
        "litellm_content_filter",
        "ContentFilterCategoryConfig",
        {"action": "BLOCK"},
    ),
    (
        "content_filter.missing_action",
        "litellm_content_filter",
        "ContentFilterCategoryConfig",
        {"category": "category"},
    ),
    (
        "content_filter.threshold_type",
        "litellm_content_filter",
        "LitellmContentFilterGuardrailConfigModel",
        {"severity_threshold": 5},
    ),
    (
        "openai.model",
        "openai.openai_moderation",
        "OpenAIModerationGuardrailConfigModel",
        {"model": "text-moderation-stable"},
    ),
    (
        "openai.rate",
        "openai.openai_moderation",
        "OpenAIModerationGuardrailConfigModel",
        {"streaming_sampling_rate": "often"},
    ),
    (
        "model_armor.flag",
        "model_armor",
        "ModelArmorGuardrailConfigModel",
        {"fail_on_error": "maybe"},
    ),
    (
        "noma.use_v2",
        "noma",
        "NomaGuardrailConfigModel",
        {"use_v2": "sometimes"},
    ),
    (
        "noma_v2.block_failures",
        "noma",
        "NomaV2GuardrailConfigModel",
        {"block_failures": "nope"},
    ),
    ("onyx.timeout", "onyx", "OnyxGuardrailConfigModel", {"timeout": "soon"}),
    (
        "ovalix.base",
        "ovalix",
        "OvalixGuardrailConfigModel",
        {"tracker_api_base": 4},
    ),
    ("pangea.api_key", "pangea", "PangeaGuardrailConfigModel", {"api_key": 3}),
    (
        "pangea.params.recipe",
        "pangea",
        "PangeaGuardrailConfigModelOptionalParams",
        {"pangea_input_recipe": 5},
    ),
    (
        "mcp.optional_params",
        "mcp_end_user_permission",
        "MCPEndUserPermissionGuardrailConfigModel",
        {"optional_params": "nested"},
    ),
]


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


def enum_members(value):
    members = getattr(value, "__members__", {})
    return [[name, member.value] for name, member in members.items()]


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
    enums = {}
    for name in names:
        value = vars(module)[name]
        if isinstance(value, type) and issubclass(value, Enum):
            enums[name] = enum_members(value)
    aliases = {}
    for alias in ALIASES.get(module_name, ()):
        aliases[alias] = schema_fields(TypeAdapter(vars(module)[alias]).json_schema())
    return {
        "names": names,
        "kinds": {name: kind_of(vars(module)[name]) for name in names},
        "shapes": shapes,
        "types": types,
        "enums": enums,
        "aliases": aliases,
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
            if hasattr(value, "ui_friendly_name"):
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
    enums: dict[str, list[list[JsonValue]]]
    aliases: dict[str, Shape]


class SideReport(TypedDict):
    modules: dict[str, ModuleDescription]
    dumps: dict[str, str]
    ui: dict[str, str | None]
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
    assert ours["types"] == theirs["types"], module_name
    assert ours["enums"] == theirs["enums"], module_name


@pytest.mark.parametrize("module_name", MODULE_NAMES)
def test_member_names(report: FullReport, module_name: str) -> None:
    theirs = report["litellm"]["modules"][module_name]
    ours = report["betterlite"]["modules"][module_name]
    aliases: tuple[str, ...] = ()
    if module_name in ALIAS_NAMES:
        aliases = ALIAS_NAMES[module_name]
    assert set(theirs["aliases"]) == set(aliases), module_name
    assert ours["aliases"] == theirs["aliases"], module_name


SectionValue = TypeVar("SectionValue")


def assert_section_matches(
    theirs: dict[str, SectionValue],
    ours: dict[str, SectionValue],
    count: int,
) -> None:
    assert len(theirs) == count
    assert ours == theirs


def test_dump_names(report: FullReport) -> None:
    assert_section_matches(
        report["litellm"]["dumps"],
        report["betterlite"]["dumps"],
        DUMP_COUNT,
    )


def test_rejection_names(report: FullReport) -> None:
    ours = report["betterlite"]["rejections"]
    assert_section_matches(report["litellm"]["rejections"], ours, REJECTION_COUNT)
    assert ours["content_filter.action"]
    assert ours["content_filter.missing_category"]
    assert ours["openai.model"]


def test_ui_names(report: FullReport) -> None:
    ui = report["betterlite"]["ui"]
    assert_section_matches(report["litellm"]["ui"], ui, UI_COUNT)
    assert ui["model_armor.ModelArmorGuardrailConfigModel"] == "Google Cloud Model Armor"


def test_abstract_names(report: FullReport) -> None:
    theirs = report["litellm"]["abstract"]
    ours = report["betterlite"]["abstract"]
    assert_section_matches(theirs, ours, ABSTRACT_COUNT)
    assert theirs["openai.openai_moderation.BaseOpenAIModerationGuardrailConfigModel"]
    assert ours["openai.openai_moderation.BaseOpenAIModerationGuardrailConfigModel"]
    assert not ours["hiddenlayer.HiddenlayerGuardrailConfigModel"]


def test_ui_friendly_names() -> None:
    assert hiddenlayer.HiddenlayerGuardrailConfigModel.ui_friendly_name() == (
        "Hiddenlayer Guardrail"
    )
    assert javelin.JavelinGuardrailConfigModel.ui_friendly_name() == "Javelin Guardrails"
    assert onyx.OnyxGuardrailConfigModel.ui_friendly_name() == "Onyx Guardrail"
    assert ibm_detector.IBMDetectorGuardrailConfigModel.ui_friendly_name() == (
        "IBM Guardrails Detector"
    )
    assert openai_moderation.OpenAIModerationGuardrailConfigModel.ui_friendly_name() == (
        "OpenAI Moderation"
    )


def test_enum_names() -> None:
    assert [(member.name, member.value) for member in hiddenlayer.HiddenlayerAction] == [
        ("BLOCK", "Block"),
        ("REDACT", "Redact"),
    ]
    assert [(member.name, member.value) for member in hiddenlayer.HiddenlayerMessages] == [
        ("BLOCK_MESSAGE", "Blocked by Hiddenlayer."),
    ]
    assert [(member.name, member.value) for member in litellm_content_filter.DetectionType] == [
        ("PATTERN", "pattern"),
        ("BLOCKED_WORD", "blocked_word"),
        ("CATEGORY_KEYWORD", "category_keyword"),
    ]
    pattern_value: str = litellm_content_filter.DetectionType.PATTERN
    assert pattern_value == "pattern"


def test_literal_names() -> None:
    intents: tuple[JsonValue, ...] = get_args(litellm_content_filter.CompetitorIntentType)
    actions: tuple[JsonValue, ...] = get_args(litellm_content_filter.CompetitorActionHint)
    assert intents == (
        "competitor_comparison",
        "possible_competitor_comparison",
        "category_ranking",
        "log_only",
        "other",
    )
    assert actions == ("allow", "reframe", "refuse", "escalate", "log_only")


def test_typeddict_names() -> None:
    assert javelin.JavelinGuardRequest.__required_keys__ == frozenset({
        "input",
        "config",
        "metadata",
    })
    assert javelin.JavelinGuardRequest.__optional_keys__ == frozenset()
    assert lakera_ai_v2.LakeraAIRequest.__optional_keys__ == frozenset({
        "messages",
        "project_id",
        "payload",
        "breakdown",
        "metadata",
        "dev_info",
    })
    assert lakera_ai_v2.LakeraAIRequest.__required_keys__ == frozenset()


def test_default_names() -> None:
    assert hiddenlayer.HiddenlayerGuardrailConfigModel().version == HIDDENLAYER_VERSION
    assert onyx.OnyxGuardrailConfigModel().timeout is None
    config = litellm_content_filter.ContentFilterCategoryConfig(
        category="harmful_self_harm",
        action="BLOCK",
    )
    assert config.enabled is True
    assert config.severity_threshold == "medium"
    assert config.category_file is None
    ibm_params = ibm_detector.IBMDetectorOptionalParams()
    assert ibm_params.detector_params == {}
    assert ibm_params.extra_headers == {}
    assert ibm_params.block_on_detection is True
    other_params = ibm_detector.IBMDetectorOptionalParams()
    assert ibm_params.detector_params is not other_params.detector_params
    assert ibm_params.extra_headers is not other_params.extra_headers


def test_optional_params_names() -> None:
    payload: dict[str, JsonValue] = {"optional_params": {"detector_params": {"a": 1}}}
    config = ibm_detector.IBMDetectorGuardrailConfigModel.model_validate(payload)
    assert config.optional_params is not None
    dumped = config.model_dump()
    assert dumped["optional_params"] == {
        "detector_params": {"a": 1},
        "extra_headers": {},
        "score_threshold": None,
        "block_on_detection": True,
    }


def test_model_config_names() -> None:
    config = openai_moderation.OpenAIModerationGuardrailConfigModel()
    assert config.model == "omni-moderation-latest"
    assert config.api_base == "https://api.openai.com/v1"
    assert config.streaming_end_of_stream_only is False
    assert config.streaming_sampling_rate == STREAMING_SAMPLING_RATE
    payload: dict[str, JsonValue] = {"model": "text-moderation-stable"}
    with pytest.raises(ValidationError):
        openai_moderation.OpenAIModerationGuardrailConfigModel.model_validate(payload)


def test_extra_field_names() -> None:
    payload: dict[str, JsonValue] = {"unknown_knob": True}
    config = onyx.OnyxGuardrailConfigModel.model_validate(payload)
    assert config.model_dump()["unknown_knob"] is True


def test_json_value_names() -> None:
    payload = {"metadata": {"not": ...}}
    with pytest.raises(ValidationError):
        javelin.JavelinGuardrailConfigModel.model_validate(payload)


def test_optional_model_names() -> None:
    config = javelin.JavelinGuardrailConfigModel()
    assert config.optional_params is None
    payload: dict[str, JsonValue] = {"optional_params": {"unknown_knob": True}}
    filled = javelin.JavelinGuardrailConfigModel.model_validate(payload)
    assert filled.optional_params is not None
    assert filled.model_dump()["optional_params"] == {}


def test_description_absent_names() -> None:
    fields = onyx.OnyxGuardrailConfigModel.model_fields
    assert all(field.description is None for field in fields.values())
