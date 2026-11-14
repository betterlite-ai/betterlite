import subprocess
import sys
from typing import TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter

from betterlite.types.guardrails import core, pii, vendor_configs

DESCRIBE_SCRIPT = """
import json
import os

os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"

from enum import Enum

import typing_extensions
from pydantic import BaseModel, TypeAdapter, ValidationError

import litellm.constants as litellm_constants
import litellm.types.guardrails as theirs

import betterlite.types.guardrails.core as our_core
import betterlite.types.guardrails.pii as our_pii
import betterlite.types.guardrails.vendor_configs as our_vendor


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


def shape(schema):
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
        "types": {name: type_shape(field) for name, field in properties.items()},
    }


def model_shape(model):
    described = shape(model.model_json_schema())
    described["bases"] = sorted(
        base.__name__
        for base in model.__bases__
        if base.__module__ == model.__module__ and not base.__name__.startswith("_")
    )
    return described


def describe(module, names, aliases):
    found = {"models": {}, "typeddicts": {}, "enums": {}, "aliases": {}}
    for name in names:
        value = vars(module).get(name)
        if value is None:
            continue
        if isinstance(value, type) and value.__module__ == module.__name__:
            if issubclass(value, BaseModel):
                found["models"][name] = model_shape(value)
            elif issubclass(value, Enum):
                found["enums"][name] = {
                    "names": [member.name for member in value],
                    "schema": shape(TypeAdapter(value).json_schema()),
                }
            elif typing_extensions.is_typeddict(value):
                found["typeddicts"][name] = shape(TypeAdapter(value).json_schema())
    for name in aliases:
        found["aliases"][name] = (
            shape(TypeAdapter(vars(module)[name]).json_schema())
            if name in vars(module)
            else None
        )
    return found


def defined(module):
    return sorted(
        name
        for name, value in vars(module).items()
        if not name.startswith("_")
        and getattr(value, "__module__", None) == module.__name__
    )


def dumps(model):
    return json.dumps(model.model_dump(), sort_keys=True, default=str)


VENDOR_ALIASES = [
    "BedrockChecksContentFilterCategory",
    "BedrockChecksPromptAttackCategory",
    "BedrockChecksSensitiveInformationEntity",
]
GROUPS = {
    "core": (our_core, []),
    "pii": (our_pii, []),
    "vendor": (our_vendor, VENDOR_ALIASES),
}
report = {"litellm": {}, "betterlite": {}}
for group, (module, aliases) in GROUPS.items():
    report["litellm"][group] = describe(theirs, defined(module), aliases)
    report["betterlite"][group] = describe(module, defined(module), aliases)

OUR_MODELS = {
    "GuardrailItem": our_core.GuardrailItem,
    "PresidioConfigModel": our_pii.PresidioConfigModel,
    "BedrockChecksConfigModel": our_vendor.BedrockChecksConfigModel,
    "BedrockGuardrailConfigModel": our_vendor.BedrockGuardrailConfigModel,
    "BedrockGuardrailStreamingParams": our_vendor.BedrockGuardrailStreamingParams,
    "LakeraV2GuardrailConfigModel": our_vendor.LakeraV2GuardrailConfigModel,
    "LassoGuardrailConfigModel": our_vendor.LassoGuardrailConfigModel,
    "DeepKeepGuardrailConfigModel": our_vendor.DeepKeepGuardrailConfigModel,
    "PillarGuardrailConfigModel": our_vendor.PillarGuardrailConfigModel,
    "NomaGuardrailConfigModel": our_vendor.NomaGuardrailConfigModel,
    "ZscalerAIGuardConfigModel": our_vendor.ZscalerAIGuardConfigModel,
    "JavelinGuardrailConfigModel": our_vendor.JavelinGuardrailConfigModel,
}
THEIR_MODELS = {
    "GuardrailItem": theirs.GuardrailItem,
    "PresidioConfigModel": theirs.PresidioConfigModel,
    "BedrockChecksConfigModel": theirs.BedrockChecksConfigModel,
    "BedrockGuardrailConfigModel": theirs.BedrockGuardrailConfigModel,
    "BedrockGuardrailStreamingParams": theirs.BedrockGuardrailStreamingParams,
    "LakeraV2GuardrailConfigModel": theirs.LakeraV2GuardrailConfigModel,
    "LassoGuardrailConfigModel": theirs.LassoGuardrailConfigModel,
    "DeepKeepGuardrailConfigModel": theirs.DeepKeepGuardrailConfigModel,
    "PillarGuardrailConfigModel": theirs.PillarGuardrailConfigModel,
    "NomaGuardrailConfigModel": theirs.NomaGuardrailConfigModel,
    "ZscalerAIGuardConfigModel": theirs.ZscalerAIGuardConfigModel,
    "JavelinGuardrailConfigModel": theirs.JavelinGuardrailConfigModel,
}
PRESIDIO_CONFIGURED = {
    "presidio_filter_scope": "input",
    "pii_entities_config": {"EMAIL_ADDRESS": "MASK"},
    "presidio_score_thresholds": {"PERSON": 0.7},
    "presidio_entities_deny_list": ["US_DRIVER_LICENSE"],
    "mock_redacted_text": {"text": "redacted"},
    "presidio_language": None,
}
BEDROCK_CONFIGURED = {
    "guardrailIdentifier": "gr-1",
    "guardrailVersion": "DRAFT",
    "checks": {"contentFilter": {"categories": [{"category": "HATE"}]}},
    "content_filter_threshold": None,
    "chunk_budget_chars": 7,
    "contextual_grounding_from_messages": True,
}
GUARDRAIL_ITEM_CONFIGURED = {
    "callbacks": ["cb"],
    "guardrail_name": "n",
    "default_on": True,
    "logging_only": True,
    "enabled_roles": ["user"],
    "callback_args": {"a": {"b": 1}},
}
SAMPLES = [
    ("guardrail_item_default", "GuardrailItem", {"callbacks": ["cb"], "guardrail_name": "n"}),
    ("guardrail_item_configured", "GuardrailItem", GUARDRAIL_ITEM_CONFIGURED),
    ("presidio_default", "PresidioConfigModel", {}),
    ("presidio_configured", "PresidioConfigModel", PRESIDIO_CONFIGURED),
    ("bedrock_default", "BedrockGuardrailConfigModel", {}),
    ("bedrock_configured", "BedrockGuardrailConfigModel", BEDROCK_CONFIGURED),
    (
        "checks_prompt_attack",
        "BedrockChecksConfigModel",
        {"promptAttack": {"categories": [{"category": "JAILBREAK"}]}},
    ),
    ("streaming_default", "BedrockGuardrailStreamingParams", {}),
    ("lakera_v2", "LakeraV2GuardrailConfigModel", {}),
    ("lasso", "LassoGuardrailConfigModel", {}),
    ("deepkeep", "DeepKeepGuardrailConfigModel", {}),
    ("pillar", "PillarGuardrailConfigModel", {}),
    ("noma", "NomaGuardrailConfigModel", {}),
    ("zscaler", "ZscalerAIGuardConfigModel", {}),
    ("javelin", "JavelinGuardrailConfigModel", {}),
]
EXTRAS = {"streaming_sampling_rate": 3, "streaming_end_of_stream_only": None, "unknown": 5}
report["dumps"] = {
    "litellm": {label: dumps(THEIR_MODELS[name](**kwargs)) for label, name, kwargs in SAMPLES},
    "betterlite": {
        label: dumps(OUR_MODELS[name](**kwargs)) for label, name, kwargs in SAMPLES
    },
}
report["dumps"]["litellm"]["streaming_extras"] = dumps(
    theirs.BedrockGuardrailStreamingParams.from_extras(EXTRAS)
)
report["dumps"]["betterlite"]["streaming_extras"] = dumps(
    our_vendor.BedrockGuardrailStreamingParams.from_extras(EXTRAS)
)


def rejection(model, kwargs):
    try:
        model(**kwargs)
    except ValidationError as error:
        return sorted([item["type"], item["msg"]] for item in error.errors())
    return []


REJECTIONS = [
    ("checks_empty", "BedrockChecksConfigModel", {}),
    (
        "checks_bad_category",
        "BedrockChecksConfigModel",
        {"contentFilter": {"categories": [{"category": "NOPE"}]}},
    ),
    ("threshold_high", "BedrockGuardrailConfigModel", {"content_filter_threshold": 1.5}),
    ("threshold_low", "BedrockGuardrailConfigModel", {"prompt_attack_threshold": -0.1}),
    ("threshold_edge_high", "BedrockGuardrailConfigModel", {"content_filter_threshold": 1.0}),
    ("threshold_edge_low", "BedrockGuardrailConfigModel", {"pii_confidence_threshold": 0.0}),
    ("chunk_zero", "BedrockGuardrailConfigModel", {"chunk_budget_chars": 0}),
    ("chunk_one", "BedrockGuardrailConfigModel", {"chunk_budget_chars": 1}),
    ("sampling_zero", "BedrockGuardrailStreamingParams", {"streaming_sampling_rate": 0}),
    ("sampling_one", "BedrockGuardrailStreamingParams", {"streaming_sampling_rate": 1}),
    ("flagged_bad", "LakeraV2GuardrailConfigModel", {"on_flagged": "warn"}),
    ("flagged_monitor", "LakeraV2GuardrailConfigModel", {"on_flagged": "monitor"}),
]
report["rejections"] = {
    "litellm": {
        label: rejection(THEIR_MODELS[name], kwargs) for label, name, kwargs in REJECTIONS
    },
    "betterlite": {
        label: rejection(OUR_MODELS[name], kwargs) for label, name, kwargs in REJECTIONS
    },
}
report["constants"] = {
    "litellm": {
        "default_roles": [role.value for role in theirs.default_roles],
        "categories": [
            {"category": key.value, "entities": [item.value for item in value]}
            for key, value in theirs.PII_ENTITY_CATEGORIES_MAP.items()
        ],
        "chunk_budget": litellm_constants.BEDROCK_APPLY_GUARDRAIL_CHUNK_BUDGET_CHARS,
    },
    "betterlite": {
        "default_roles": [role.value for role in our_core.default_roles],
        "categories": [
            {"category": key.value, "entities": [item.value for item in value]}
            for key, value in our_pii.PII_ENTITY_CATEGORIES_MAP.items()
        ],
        "chunk_budget": our_vendor.BEDROCK_APPLY_GUARDRAIL_CHUNK_BUDGET_CHARS,
    },
}
print(json.dumps(report))
"""

CORE_MODELS = frozenset({"GuardrailItem"})
CORE_ENUMS = frozenset({"SupportedGuardrailIntegrations", "Role"})
CORE_TYPEDDICTS = frozenset({"GuardrailItemSpec", "LakeraCategoryThresholds"})
PII_MODELS = frozenset({"PresidioPresidioConfigModelUserInterface", "PresidioConfigModel"})
PII_ENUMS = frozenset(
    {"PiiAction", "PiiEntityCategory", "PiiEntityType", "GuardrailParamUITypes"}
)
PII_TYPEDDICTS = frozenset({"PiiEntityCategoryMap"})
VENDOR_MODELS = frozenset({
    "BedrockChecksContentFilterCategoryItem",
    "BedrockChecksContentFilterModel",
    "BedrockChecksPromptAttackCategoryItem",
    "BedrockChecksPromptAttackModel",
    "BedrockChecksSensitiveInformationEntityItem",
    "BedrockChecksSensitiveInformationModel",
    "BedrockChecksConfigModel",
    "BedrockGuardrailConfigModel",
    "BedrockGuardrailStreamingParams",
    "LakeraV2GuardrailConfigModel",
    "LassoGuardrailConfigModel",
    "DeepKeepGuardrailConfigModel",
    "PillarGuardrailConfigModel",
    "NomaGuardrailConfigModel",
    "ZscalerAIGuardConfigModel",
    "JavelinGuardrailConfigModel",
})
VENDOR_ALIASES = frozenset({
    "BedrockChecksContentFilterCategory",
    "BedrockChecksPromptAttackCategory",
    "BedrockChecksSensitiveInformationEntity",
})
CATEGORY_ORDER = [
    "General", "Finance", "USA", "UK", "Spain", "Italy", "Poland", "Singapore",
    "Australia", "India", "Finland", "Germany", "Korea", "Canada", "Sweden",
    "Thailand", "Turkey", "Nigeria", "Philippines", "South Africa",
]
GENERAL_ENTITIES = [
    "DATE_TIME", "EMAIL_ADDRESS", "IP_ADDRESS", "NRP", "LOCATION", "PERSON",
    "PHONE_NUMBER", "MEDICAL_LICENSE", "URL", "MAC_ADDRESS", "UUID",
]
CHUNK_BUDGET_CHARS = 25000
ENTITY_COUNT = 79
DEFAULT_SAMPLING_RATE = 5
SAMPLING_RATE = 3


class Shape(TypedDict):
    keys: list[str]
    required: list[str]
    defaults: dict[str, str]
    literals: list[str]
    types: dict[str, JsonValue]


class ModelShape(Shape):
    bases: list[str]


class EnumShape(TypedDict):
    names: list[str]
    schema: Shape


class ModuleNames(TypedDict):
    models: dict[str, ModelShape]
    typeddicts: dict[str, Shape]
    enums: dict[str, EnumShape]
    aliases: dict[str, Shape | None]


class GroupNames(TypedDict):
    core: ModuleNames
    pii: ModuleNames
    vendor: ModuleNames


class CategoryEntry(TypedDict):
    category: str
    entities: list[str]


class Constants(TypedDict):
    default_roles: list[str]
    categories: list[CategoryEntry]
    chunk_budget: int


class NamesReport(TypedDict):
    litellm: GroupNames
    betterlite: GroupNames
    dumps: dict[str, dict[str, str]]
    rejections: dict[str, dict[str, list[list[str]]]]
    constants: dict[str, Constants]


@pytest.fixture(scope="module")
def report() -> NamesReport:
    completed = subprocess.run(
        [sys.executable, "-c", DESCRIBE_SCRIPT],
        check=True,
        capture_output=True,
        text=True,
    )
    return TypeAdapter(NamesReport).validate_json(completed.stdout)


def test_core_names(report: NamesReport) -> None:
    expected = report["litellm"]["core"]
    assert set(expected["models"]) == CORE_MODELS
    assert set(expected["enums"]) == CORE_ENUMS
    assert set(expected["typeddicts"]) == CORE_TYPEDDICTS
    assert not expected["aliases"]
    assert report["betterlite"]["core"] == expected


def test_pii_names(report: NamesReport) -> None:
    expected = report["litellm"]["pii"]
    assert set(expected["models"]) == PII_MODELS
    assert set(expected["enums"]) == PII_ENUMS
    assert set(expected["typeddicts"]) == PII_TYPEDDICTS
    assert not expected["aliases"]
    assert report["betterlite"]["pii"] == expected


def test_vendor_config_names(report: NamesReport) -> None:
    expected = report["litellm"]["vendor"]
    assert set(expected["models"]) == VENDOR_MODELS
    assert not expected["enums"]
    assert not expected["typeddicts"]
    assert set(expected["aliases"]) == VENDOR_ALIASES
    assert report["betterlite"]["vendor"] == expected


def test_constant_names(report: NamesReport) -> None:
    theirs = report["litellm"]["constants"]
    ours = report["betterlite"]["constants"]
    assert ours == theirs
    assert ours["default_roles"] == ["system", "assistant", "user"]
    assert ours["chunk_budget"] == CHUNK_BUDGET_CHARS
    assert [entry["category"] for entry in ours["categories"]] == CATEGORY_ORDER
    assert ours["categories"][0]["entities"] == GENERAL_ENTITIES


def test_model_dump_names(report: NamesReport) -> None:
    dumps = report["dumps"]
    assert dumps["betterlite"] == dumps["litellm"]


def test_rejection_names(report: NamesReport) -> None:
    rejections = report["rejections"]
    assert rejections["betterlite"] == rejections["litellm"]


def test_pii_map_names() -> None:
    general = pii.PII_ENTITY_CATEGORIES_MAP[pii.PiiEntityCategory.GENERAL]
    assert list(general) == [
        pii.PiiEntityType.DATE_TIME,
        pii.PiiEntityType.EMAIL_ADDRESS,
        pii.PiiEntityType.IP_ADDRESS,
        pii.PiiEntityType.NRP,
        pii.PiiEntityType.LOCATION,
        pii.PiiEntityType.PERSON,
        pii.PiiEntityType.PHONE_NUMBER,
        pii.PiiEntityType.MEDICAL_LICENSE,
        pii.PiiEntityType.URL,
        pii.PiiEntityType.MAC_ADDRESS,
        pii.PiiEntityType.UUID,
    ]
    mapped = [
        entity for entities in pii.PII_ENTITY_CATEGORIES_MAP.values() for entity in entities
    ]
    assert len(mapped) == ENTITY_COUNT
    assert len(set(mapped)) == ENTITY_COUNT
    assert len(pii.PII_ENTITY_CATEGORIES_MAP) == len(CATEGORY_ORDER)


def test_guardrail_item_names() -> None:
    item = core.GuardrailItem(callbacks=["cb"], guardrail_name="n")
    expected: dict[str, JsonValue] = {
        "callbacks": ["cb"],
        "default_on": False,
        "logging_only": None,
        "guardrail_name": "n",
        "callback_args": {},
        "enabled_roles": ["system", "assistant", "user"],
    }
    assert item.model_dump() == expected
    roles = item.enabled_roles
    assert roles is not None
    assert roles[0] is not core.Role.SYSTEM
    assert core.GuardrailItem(callbacks=["cb"], guardrail_name="n", callback_args={}) == item
    assert core.GuardrailItem(callbacks=["cb"], guardrail_name="n", callback_args={"a": {}}) != item


def test_presidio_ui_names() -> None:
    field = pii.PresidioPresidioConfigModelUserInterface.model_fields["output_parse_pii"]
    assert field.json_schema_extra == {"ui_type": "bool"}


def test_bedrock_alias_names() -> None:
    config = vendor_configs.BedrockGuardrailConfigModel(
        guardrailIdentifier="gr-1",
        checks={"contentFilter": {"categories": [{"category": "HATE"}]}},
    )
    assert config.guardrail_identifier == "gr-1"
    assert config.guardrail_version is None
    checks = config.checks
    assert checks is not None
    assert checks.content_filter is not None
    expected: dict[str, JsonValue] = {
        "contentFilter": {"categories": [{"category": "HATE"}]},
        "promptAttack": None,
        "sensitiveInformation": None,
    }
    dumped = config.model_dump()
    assert dumped["guardrailIdentifier"] == "gr-1"
    assert dumped["guardrailVersion"] is None
    assert dumped["checks"] == expected


def test_extra_field_names() -> None:
    config = vendor_configs.BedrockGuardrailConfigModel(
        guardrailIdentifier="gr-1",
        unknown_knob=True,
    )
    assert "unknown_knob" in config.model_dump()


def test_streaming_extra_names() -> None:
    params = vendor_configs.BedrockGuardrailStreamingParams.from_extras(
        {
            "streaming_sampling_rate": SAMPLING_RATE,
            "streaming_end_of_stream_only": None,
            "unknown": 5,
        }
    )
    assert params.streaming_sampling_rate == SAMPLING_RATE
    assert params.streaming_end_of_stream_only is False
    assert params.streaming_buffer_until_moderated is True
    assert "unknown" not in params.model_dump()
    defaults = vendor_configs.BedrockGuardrailStreamingParams.from_extras(None)
    assert defaults.streaming_sampling_rate == DEFAULT_SAMPLING_RATE
