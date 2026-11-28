import subprocess
import sys
from typing import Final, TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter, ValidationError

from betterlite.types.proxy.guardrails.guardrail_hooks import (
    conduct,
    crowdstrike_aidr,
    deepkeep,
    dynamoai,
    enkryptai,
    generic_guardrail_api,
    grayswan,
    guardrails_ai,
)

MODULE_NAMES: Final = (
    "compresr",
    "conduct",
    "crowdstrike_aidr",
    "decision_model",
    "deepkeep",
    "dynamoai",
    "enkryptai",
    "generic_guardrail_api",
    "grayswan",
    "guardrails_ai",
    "headroom",
)
# Recorded from the pinned LiteLLM files, one entry per module
NAME_COUNTS: Final = {
    "compresr": 2,
    "conduct": 2,
    "crowdstrike_aidr": 2,
    "decision_model": 2,
    "deepkeep": 2,
    "dynamoai": 12,
    "enkryptai": 11,
    "generic_guardrail_api": 8,
    "grayswan": 2,
    "guardrails_ai": 1,
    "headroom": 1,
}
DUMP_COUNT: Final = 36
REJECTION_COUNT: Final = 20
UI_COUNT: Final = 11
ABSTRACT_COUNT: Final = 21
COERCE_COUNT: Final = 18
STRUCTURED_COUNT: Final = 12
RESPONSE_COUNT: Final = 10
DESCRIBE_SCRIPT = """
import importlib
import inspect
import json
import os

os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"

from enum import Enum

from pydantic import BaseModel, TypeAdapter, ValidationError
from typing_extensions import is_typeddict

PACKAGES = {
    "litellm": "litellm.types.proxy.guardrails.guardrail_hooks",
    "betterlite": "betterlite.types.proxy.guardrails.guardrail_hooks",
}
MODULES = (
    "compresr",
    "conduct",
    "crowdstrike_aidr",
    "decision_model",
    "deepkeep",
    "dynamoai",
    "enkryptai",
    "generic_guardrail_api",
    "grayswan",
    "guardrails_ai",
    "headroom",
)
CONSTRAINT_KEYS = (
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "minLength",
    "maxLength",
    "pattern",
    "multipleOf",
    "minItems",
    "maxItems",
)
UI_EXTRA_KEYS = (
    "ui_type",
    "options",
    "min",
    "max",
    "step",
    "default_value",
    "examples",
    "secret",
)
SAMPLES = [
    ("compresr.default", "compresr", "CompresrGuardrailConfigModel", {}),
    (
        "compresr.configured", "compresr", "CompresrGuardrailConfigModel",
        {
            "api_key": "key", "api_base": "https://compresr.example", "model": "latte_v2",
            "unreachable_fallback": "fail_open", "optional_params": {"coarse": False},
        },
    ),
    ("compresr.params.default", "compresr", "CompresrGuardrailOptionalParams", {}),
    (
        "compresr.params.configured", "compresr", "CompresrGuardrailOptionalParams",
        {
            "target_compression_ratio": 0.5, "coarse": True, "min_chars_to_compress": 10,
            "compress_tool_outputs": False, "compress_system": True,
            "compress_history": True, "compress_last_user": True,
            "enable_retrieval": False, "max_bytes_per_call": 1024,
            "allow_bypass_header": True, "dynamic": False, "dynamic_min_ratio": 1.5,
            "dynamic_max_ratio": 10.0, "compression_params": {"heuristic_chunking": True},
        },
    ),
    ("conduct.params.default", "conduct", "ConductGuardrailConfigModelOptionalParams", {}),
    (
        "conduct.params.configured", "conduct", "ConductGuardrailConfigModelOptionalParams",
        {
            "workspace_id": "workspace", "tool_name": "chat", "timeout": 2.5,
            "unreachable_fallback": "fail_open",
        },
    ),
    (
        "conduct.config.configured", "conduct", "ConductGuardrailConfigModel",
        {"api_key": "token", "api_base": "https://conduct.example", "optional_params": {}},
    ),
    ("crowdstrike.params.default", "crowdstrike_aidr",
     "CrowdStrikeAIDRGuardrailConfigModelOptionalParams", {}),
    (
        "crowdstrike.params.configured", "crowdstrike_aidr",
        "CrowdStrikeAIDRGuardrailConfigModelOptionalParams",
        {
            "streaming_buffer_until_moderated": True,
            "streaming_buffer_release_on_scan": True,
            "streaming_end_of_stream_only": False, "streaming_sampling_rate": 3,
        },
    ),
    (
        "crowdstrike.config.configured", "crowdstrike_aidr",
        "CrowdStrikeAIDRGuardrailConfigModel",
        {
            "api_key": "key", "api_base": "https://crowdstrike.example",
            "fail_on_error": False, "optional_params": {"streaming_sampling_rate": 2},
        },
    ),
    (
        "decision.check.default", "decision_model", "DecisionModelCheck",
        {"name": "harm", "instructions": "flag harm"},
    ),
    (
        "decision.check.configured", "decision_model", "DecisionModelCheck",
        {"name": "pii", "instructions": "no pii", "threshold": 0.2, "action": "log"},
    ),
    (
        "decision.config.default", "decision_model", "DecisionModelGuardrailConfigModel",
        {"decision_model": "judge", "checks": [{"name": "harm", "instructions": "flag harm"}]},
    ),
    (
        "decision.config.configured", "decision_model", "DecisionModelGuardrailConfigModel",
        {
            "decision_model": "judge",
            "checks": [{"name": "harm", "instructions": "flag harm", "threshold": 0.9}],
            "max_input_chars": 1000, "max_concurrent_decision_calls": 2,
            "streaming_end_of_stream_only": True, "streaming_sampling_rate": 3,
        },
    ),
    ("deepkeep.params.default", "deepkeep",
     "DeepKeepGuardrailConfigModelOptionalParams", {}),
    (
        "deepkeep.config.configured", "deepkeep", "DeepKeepGuardrailConfigModel",
        {
            "api_key": "key", "api_base": "https://deepkeep.example",
            "deepkeep_firewall_id": "firewall",
            "optional_params": {"unreachable_fallback": "fail_open"},
        },
    ),
    ("dynamoai.config.default", "dynamoai", "DynamoAIGuardrailConfigModel", {}),
    (
        "dynamoai.config.configured", "dynamoai", "DynamoAIGuardrailConfigModel",
        {
            "api_key": "key", "api_base": "https://dynamo.example",
            "policy_ids": ["policy"], "model_id": "model", "guardrail_name": "guard",
        },
    ),
    ("enkryptai.configs.default", "enkryptai", "EnkryptAIGuardrailConfigs", {}),
    (
        "enkryptai.configs.configured", "enkryptai", "EnkryptAIGuardrailConfigs",
        {
            "api_key": "key", "api_base": "https://enkrypt.example",
            "policy_name": "policy", "deployment_name": "deployment",
            "detectors": {"nsfw": {"enabled": True}}, "block_on_violation": False,
        },
    ),
    ("enkryptai.config.default", "enkryptai", "EnkryptAIGuardrailConfigModel", {}),
    (
        "enkryptai.config.configured", "enkryptai", "EnkryptAIGuardrailConfigModel",
        {"api_key": "key", "policy_name": "policy", "block_on_violation": False},
    ),
    (
        "generic.tool.default", "generic_guardrail_api", "GuardrailToolParam",
        {"type": "function"},
    ),
    (
        "generic.tool.configured", "generic_guardrail_api", "GuardrailToolParam",
        {"type": "code_interpreter", "container": {"type": "auto"}},
    ),
    ("generic.params.default", "generic_guardrail_api",
     "GenericGuardrailAPIOptionalParams", {}),
    (
        "generic.params.configured", "generic_guardrail_api",
        "GenericGuardrailAPIOptionalParams",
        {
            "additional_provider_specific_params": {"knob": 1},
            "unreachable_fallback": "fail_open", "fail_on_error": False,
            "streaming_end_of_stream_only": True, "streaming_sampling_rate": 2,
            "streaming_transform_mode": "incremental_diff",
        },
    ),
    ("generic.config.default", "generic_guardrail_api",
     "GenericGuardrailAPIConfigModel", {}),
    (
        "generic.config.configured", "generic_guardrail_api",
        "GenericGuardrailAPIConfigModel",
        {"optional_params": {"fail_on_error": False}},
    ),
    (
        "generic.request.full", "generic_guardrail_api", "GenericGuardrailAPIRequest",
        {
            "input_type": "request", "litellm_call_id": "call-1",
            "litellm_trace_id": "trace-1",
            "structured_messages": [{"role": "user", "content": "hello"}],
            "images": ["https://img.example/one.png"],
            "tools": [{"type": "function", "name": "lookup"}],
            "texts": ["hello"], "request_data": {"user_api_key_hash": "hash"},
            "request_headers": {"x-custom": "one"}, "litellm_version": "1.105.1",
            "additional_provider_specific_params": {"knob": 1},
            "tool_calls": [
                {"id": "call_1", "type": "function", "index": 0,
                 "function": {"name": "lookup", "arguments": "{}"}},
            ],
            "model": "gpt-4o",
        },
    ),
    ("grayswan.params.default", "grayswan",
     "GraySwanGuardrailConfigModelOptionalParams", {}),
    (
        "grayswan.params.configured", "grayswan",
        "GraySwanGuardrailConfigModelOptionalParams",
        {
            "on_flagged_action": "block", "violation_threshold": 0.8,
            "reasoning_mode": "hybrid", "policy_id": "policy",
            "categories": {"jailbreak": "high"}, "fail_open": False,
            "guardrail_timeout": 5.0,
        },
    ),
    (
        "grayswan.config.configured", "grayswan", "GraySwanGuardrailConfigModel",
        {"api_key": "key", "api_base": "https://grayswan.example", "optional_params": {}},
    ),
    ("guardrails_ai.config.default", "guardrails_ai",
     "GuardrailsAIGuardrailConfigModel", {}),
    (
        "guardrails_ai.config.configured", "guardrails_ai",
        "GuardrailsAIGuardrailConfigModel",
        {
            "api_base": "http://guardrails:8000", "guard_name": "guard",
            "guardrails_ai_api_input_format": "inputs",
        },
    ),
    ("headroom.config.default", "headroom", "HeadroomGuardrailConfigModel", {}),
    (
        "headroom.config.configured", "headroom", "HeadroomGuardrailConfigModel",
        {
            "api_base": "https://headroom.example", "api_key": "key", "model": "compress",
            "unreachable_fallback": "fail_open", "ccr_retrieval": False,
        },
    ),
]
REJECTIONS = [
    ("compresr.fallback", "compresr", "CompresrGuardrailConfigModel",
     {"unreachable_fallback": "open"}),
    ("conduct.api_key_missing", "conduct", "ConductGuardrailConfigModel", {}),
    ("conduct.api_key_empty", "conduct", "ConductGuardrailConfigModel", {"api_key": ""}),
    ("conduct.timeout", "conduct", "ConductGuardrailConfigModelOptionalParams",
     {"timeout": 0.0}),
    ("crowdstrike.sampling_rate", "crowdstrike_aidr",
     "CrowdStrikeAIDRGuardrailConfigModelOptionalParams",
     {"streaming_sampling_rate": 0}),
    ("decision.threshold", "decision_model", "DecisionModelCheck",
     {"name": "harm", "instructions": "flag harm", "threshold": 1.5}),
    ("decision.blank_instructions", "decision_model", "DecisionModelCheck",
     {"name": "harm", "instructions": "   "}),
    ("decision.action", "decision_model", "DecisionModelCheck",
     {"name": "harm", "instructions": "flag harm", "action": "warn"}),
    ("decision.no_checks", "decision_model", "DecisionModelGuardrailConfigModel",
     {"decision_model": "judge", "checks": []}),
    ("decision.max_input", "decision_model", "DecisionModelGuardrailConfigModel",
     {
         "decision_model": "judge",
         "checks": [{"name": "harm", "instructions": "flag harm"}],
         "max_input_chars": 0,
     }),
    ("deepkeep.fallback", "deepkeep", "DeepKeepGuardrailConfigModelOptionalParams",
     {"unreachable_fallback": 3}),
    ("dynamoai.policy_ids", "dynamoai", "DynamoAIGuardrailConfigModel",
     {"policy_ids": "policy"}),
    ("enkryptai.block_on_violation", "enkryptai", "EnkryptAIGuardrailConfigs",
     {"block_on_violation": 3}),
    ("generic.input_type", "generic_guardrail_api", "GenericGuardrailAPIRequest",
     {"input_type": "other", "request_data": {}}),
    ("generic.sampling_rate", "generic_guardrail_api",
     "GenericGuardrailAPIOptionalParams",
     {"streaming_sampling_rate": 0}),
    ("generic.tool_missing_type", "generic_guardrail_api", "GuardrailToolParam", {}),
    ("generic.transform_mode", "generic_guardrail_api",
     "GenericGuardrailAPIOptionalParams",
     {"streaming_transform_mode": "block"}),
    ("grayswan.threshold", "grayswan", "GraySwanGuardrailConfigModelOptionalParams",
     {"violation_threshold": 1.5}),
    ("guardrails_ai.input_format", "guardrails_ai",
     "GuardrailsAIGuardrailConfigModel",
     {"guardrails_ai_api_input_format": "messages"}),
    ("headroom.fallback", "headroom", "HeadroomGuardrailConfigModel",
     {"unreachable_fallback": "open"}),
]
COERCE_INPUTS = [
    ("null", None),
    ("true", True),
    ("false", False),
    ("zero", 0),
    ("positive", 7),
    ("negative", -3),
    ("float_down", 2.9),
    ("float_negative", -0.5),
    ("string", "12"),
    ("string_spaces", " 42 "),
    ("string_negative", "-4"),
    ("string_float", "1.5"),
    ("string_text", "abc"),
    ("string_empty", ""),
    ("string_underscores", "1_000"),
    ("list", [3]),
    ("object", {"chars": 3}),
    ("huge", 10 ** 30),
]
STRUCTURED_INPUTS = [
    ("none", None),
    ("text", "hello"),
    ("number", 3),
    ("empty_list", []),
    ("user", [{"role": "user", "content": "hello"}]),
    ("assistant", [{"role": "assistant", "content": None}]),
    ("system", [{"role": "system", "content": "rules"}]),
    ("extra_key", [{"role": "user", "content": "hello", "name": "ada"}]),
    ("custom_role", [{"role": "custom", "content": "hello"}]),
    ("no_role", [{"content": "hello"}]),
    ("role_number", [{"role": 3, "content": "hello"}]),
    ("mixed", [{"role": "user", "content": "hello"}, {"role": "assistant", "content": None}]),
]
RESPONSE_INPUTS = [
    ("empty", {}),
    ("action", {"action": "BLOCKED"}),
    ("empty_action", {"action": ""}),
    ("blocked", {"action": "BLOCKED", "blocked_reason": "policy"}),
    ("texts", {"action": "NONE", "texts": ["a", "b"], "images": ["u"]}),
    ("holdback", {"action": "X", "stream_holdback_chars": [3, -1, "2", "x", None, True]}),
    ("holdback_scalar", {"action": "NONE", "stream_holdback_chars": 5}),
    ("tools", {"tools": [{"type": "function", "name": "lookup"}]}),
    ("messages", {"structured_messages": [{"role": "user", "content": "hello"}]}),
    ("extra_key", {"action": "NONE", "other": {"nested": 1}}),
]


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
        shape = {"array": type_shape(node.get("items", {}))}
    elif kind is None:
        return "json"
    else:
        shape = kind
    constraints = {key: node[key] for key in CONSTRAINT_KEYS if key in node}
    if constraints:
        return {"shape": shape, "constraints": constraints}
    return shape


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
    order = {}
    frozen = {}
    for name in names:
        value = vars(module)[name]
        if is_typeddict(value):
            schema = TypeAdapter(value).json_schema()
            order[name] = list(value.__annotations__)
        elif isinstance(value, type) and issubclass(value, BaseModel):
            schema = value.model_json_schema()
            order[name] = list(value.model_fields)
            frozen[name] = value.model_config.get("frozen")
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
        "order": order,
        "frozen": frozen,
        "ui_extras": extras,
    }


def model_value(package, module_name, model_name, kwargs):
    module = importlib.import_module(package + "." + module_name)
    return vars(module)[model_name](**kwargs)


def response_fields(response):
    tools = response.tools
    if tools is not None:
        tools = [
            tool.model_dump() if isinstance(tool, BaseModel) else tool
            for tool in tools
        ]
    messages = response.structured_messages
    if messages is not None:
        messages = [dict(message) for message in messages]
    return {
        "action": response.action,
        "blocked_reason": response.blocked_reason,
        "texts": response.texts,
        "images": response.images,
        "tools": tools,
        "stream_holdback_chars": response.stream_holdback_chars,
        "structured_messages": messages,
    }


def open_json_report(module):
    try:
        model = module.GenericGuardrailAPIOptionalParams.model_validate(
            {"additional_provider_specific_params": {"knob": object()}},
        )
    except ValidationError as error:
        return {"errors": sorted([item["type"], str(item["loc"])] for item in error.errors())}
    return {"keys": sorted(model.model_dump())}


def function_report(module):
    coerced = {}
    for label, value in COERCE_INPUTS:
        try:
            coerced[label] = {"value": module.coerce_stream_holdback_value(value)}
        except Exception as error:
            coerced[label] = {"error": type(error).__name__}
    messages = {}
    for label, value in STRUCTURED_INPUTS:
        try:
            messages[label] = {"value": module.structured_messages_from_response(value)}
        except Exception as error:
            messages[label] = {"error": type(error).__name__}
    responses = {}
    for label, payload in RESPONSE_INPUTS:
        try:
            response = module.GenericGuardrailAPIResponse.from_dict(payload)
            responses[label] = response_fields(response)
        except Exception as error:
            responses[label] = {"error": type(error).__name__}
    return {
        "coerce": coerced,
        "structured": messages,
        "responses": responses,
        "open_json": open_json_report(module),
    }


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
    functions = function_report(importlib.import_module(package + ".generic_guardrail_api"))
    for label, module_name, model_name, kwargs in SAMPLES:
        instance = model_value(package, module_name, model_name, kwargs)
        dumps[label] = json.dumps(instance.model_dump(mode="json"), sort_keys=True)
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
        "functions": functions,
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


class FunctionReport(TypedDict):
    coerce: dict[str, JsonValue]
    structured: dict[str, JsonValue]
    responses: dict[str, JsonValue]
    open_json: JsonValue


class ModuleDescription(TypedDict):
    names: list[str]
    kinds: dict[str, str]
    shapes: dict[str, Shape]
    types: dict[str, dict[str, JsonValue]]
    order: dict[str, list[str]]
    frozen: dict[str, bool | None]
    ui_extras: dict[str, JsonValue]


class SideReport(TypedDict):
    modules: dict[str, ModuleDescription]
    dumps: dict[str, str]
    ui: dict[str, str]
    abstract: dict[str, bool]
    rejections: dict[str, list[list[str]]]
    functions: FunctionReport


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
    assert len(theirs["names"]) == NAME_COUNTS[module_name], module_name
    assert ours["names"] == theirs["names"], module_name
    assert ours["kinds"] == theirs["kinds"], module_name
    assert ours["shapes"] == theirs["shapes"], module_name
    assert ours["types"] == theirs["types"], module_name
    assert ours["order"] == theirs["order"], module_name
    assert ours["frozen"] == theirs["frozen"], module_name
    assert ours["ui_extras"] == theirs["ui_extras"], module_name


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
    assert ours["conduct.api_key_missing"] == [["missing", "Field required", "('api_key',)"]]
    assert ours["decision.blank_instructions"] == [
        ["value_error", "Value error, instructions must not be blank", "('instructions',)"],
    ]


def test_ui_names(report: FullReport) -> None:
    theirs = report["litellm"]["ui"]
    ours = report["betterlite"]["ui"]
    assert len(theirs) == UI_COUNT
    assert ours == theirs
    assert ours["compresr.CompresrGuardrailConfigModel"] == "Compresr (context compression)"
    assert ours["conduct.ConductGuardrailConfigModel"] == "Conduct Guard"


def test_abstract_names(report: FullReport) -> None:
    theirs = report["litellm"]["abstract"]
    ours = report["betterlite"]["abstract"]
    assert len(theirs) == ABSTRACT_COUNT
    assert ours == theirs
    assert ours["generic_guardrail_api.GuardrailToolParam"] is False
    assert ours["decision_model.DecisionModelCheck"] is False


def test_default_names() -> None:
    conduct_config: dict[str, JsonValue] = conduct.ConductGuardrailConfigModel(
        api_key="token",
    ).model_dump(mode="json")
    assert conduct_config == {
        "api_base": "https://api.conductai.ai", "api_key": "token", "optional_params": None,
    }
    generic_config: dict[str, JsonValue] = (
        generic_guardrail_api.GenericGuardrailAPIConfigModel().model_dump(mode="json")
    )
    assert generic_config == {
        "optional_params": {
            "additional_provider_specific_params": None, "fail_on_error": True,
            "streaming_end_of_stream_only": None, "streaming_sampling_rate": None,
            "streaming_transform_mode": None, "unreachable_fallback": "fail_closed",
        },
    }
    deepkeep_config: dict[str, JsonValue] = (
        deepkeep.DeepKeepGuardrailConfigModel().model_dump(mode="json")
    )
    assert deepkeep_config == {
        "api_base": None, "api_key": None, "deepkeep_firewall_id": None,
        "optional_params": None,
    }
    crowdstrike_config: dict[str, JsonValue] = (
        crowdstrike_aidr.CrowdStrikeAIDRGuardrailConfigModel().model_dump(mode="json")
    )
    assert crowdstrike_config == {
        "api_base": None, "api_key": None, "fail_on_error": True, "optional_params": None,
    }
    grayswan_config: dict[str, JsonValue] = grayswan.GraySwanGuardrailConfigModel().model_dump(
        mode="json",
    )
    assert grayswan_config == {
        "api_base": None, "api_key": None, "optional_params": None,
    }


def test_optional_params_names() -> None:
    enkryptai_config: dict[str, JsonValue] = (
        enkryptai.EnkryptAIGuardrailConfigModel.model_validate_json(
            '{"optional_params": {"knob": true}}',
        ).model_dump(mode="json")
    )
    assert enkryptai_config["optional_params"] == {}
    dynamoai_config: dict[str, JsonValue] = (
        dynamoai.DynamoAIGuardrailConfigModel.model_validate_json(
            '{"optional_params": {"knob": true}}',
        ).model_dump(mode="json")
    )
    assert dynamoai_config["optional_params"] == {}
    guardrails_ai_config: dict[str, JsonValue] = (
        guardrails_ai.GuardrailsAIGuardrailConfigModel.model_validate_json(
            '{"optional_params": {"knob": true}}',
        ).model_dump(mode="json")
    )
    assert guardrails_ai_config["optional_params"] == {}


def test_required_names() -> None:
    assert conduct.ConductGuardrailConfigModel.model_fields["api_key"].is_required()
    assert generic_guardrail_api.GenericGuardrailAPIRequest.model_fields[
        "input_type"
    ].is_required()
    assert generic_guardrail_api.GenericGuardrailAPIRequest.model_fields[
        "request_data"
    ].is_required()
    empty: dict[str, JsonValue] = {}
    with pytest.raises(ValidationError):
        conduct.ConductGuardrailConfigModel.model_validate(empty)


def test_enum_names() -> None:
    assert [member.value for member in dynamoai.DynamoTextType] == [
        "MODEL_INPUT", "MODEL_RESPONSE",
    ]
    assert [member.value for member in dynamoai.PolicyMethod] == [
        "PII",
        "TOXICITY",
        "ALIGNMENT",
        "HALLUCINATION",
        "RAG_HALLUCINATION",
    ]
    applicable = [member.value for member in dynamoai.PolicyApplicableTo]
    assert applicable == ["INPUT", "OUTPUT", "ALL"]
    assert dynamoai.DynamoTextType("MODEL_INPUT") is dynamoai.DynamoTextType.MODEL_INPUT
    assert str(dynamoai.DynamoTextType.MODEL_INPUT) == "DynamoTextType.MODEL_INPUT"


def test_tool_param_names() -> None:
    payload: dict[str, JsonValue] = {"type": "code_interpreter", "container": {"type": "auto"}}
    param = generic_guardrail_api.GuardrailToolParam.model_validate(payload)
    dumped = TypeAdapter(dict[str, JsonValue]).validate_json(param.model_dump_json())
    assert param.type == "code_interpreter"
    assert dumped == payload


def test_coerce_names(report: FullReport) -> None:
    theirs = report["litellm"]["functions"]["coerce"]
    ours = report["betterlite"]["functions"]["coerce"]
    assert len(theirs) == COERCE_COUNT
    assert ours == theirs
    assert ours["null"] == {"value": 0}
    assert ours["true"] == {"value": 1}
    assert ours["negative"] == {"value": 0}
    assert ours["float_down"] == {"value": 2}
    assert ours["string_spaces"] == {"value": 42}
    assert ours["string_underscores"] == {"value": 1000}
    assert ours["string_float"] == {"value": 0}
    assert ours["list"] == {"value": 0}
    true_value: JsonValue = True
    direct: dict[str, int] = {
        "null": generic_guardrail_api.coerce_stream_holdback_value(None),
        "true": generic_guardrail_api.coerce_stream_holdback_value(true_value),
        "float_down": generic_guardrail_api.coerce_stream_holdback_value(2.9),
        "negative": generic_guardrail_api.coerce_stream_holdback_value(-3),
        "spaces": generic_guardrail_api.coerce_stream_holdback_value(" 42 "),
        "underscores": generic_guardrail_api.coerce_stream_holdback_value("1_000"),
        "float_string": generic_guardrail_api.coerce_stream_holdback_value("1.5"),
        "list": generic_guardrail_api.coerce_stream_holdback_value([3]),
    }
    assert direct == {
        "null": 0,
        "true": 1,
        "float_down": 2,
        "negative": 0,
        "spaces": 42,
        "underscores": 1000,
        "float_string": 0,
        "list": 0,
    }


def test_structured_names(report: FullReport) -> None:
    theirs = report["litellm"]["functions"]["structured"]
    ours = report["betterlite"]["functions"]["structured"]
    assert len(theirs) == STRUCTURED_COUNT
    assert ours == theirs
    assert ours["none"] == {"value": None}
    assert ours["no_role"] == {"value": None}
    assert ours["role_number"] == {"value": None}
    assert ours["extra_key"] == {"value": [{"role": "user", "content": "hello", "name": "ada"}]}
    assert ours["custom_role"] == {"value": [{"role": "custom", "content": "hello"}]}


def test_open_json_names(report: FullReport) -> None:
    theirs = report["litellm"]["functions"]["open_json"]
    ours = report["betterlite"]["functions"]["open_json"]
    assert theirs == {
        "keys": [
            "additional_provider_specific_params",
            "fail_on_error",
            "streaming_end_of_stream_only",
            "streaming_sampling_rate",
            "streaming_transform_mode",
            "unreachable_fallback",
        ],
    }
    assert ours == {
        "errors": [["invalid-json-value", "('additional_provider_specific_params', 'knob')"]],
    }


def test_response_names(report: FullReport) -> None:
    theirs = report["litellm"]["functions"]["responses"]
    ours = report["betterlite"]["functions"]["responses"]
    assert len(theirs) == RESPONSE_COUNT
    assert ours == theirs
    assert ours["empty"] == {
        "action": "NONE",
        "blocked_reason": None,
        "texts": None,
        "images": None,
        "tools": None,
        "stream_holdback_chars": None,
        "structured_messages": None,
    }
    assert ours["empty_action"] == {
        "action": "",
        "blocked_reason": None,
        "texts": None,
        "images": None,
        "tools": None,
        "stream_holdback_chars": None,
        "structured_messages": None,
    }
    assert ours["holdback"] == {
        "action": "X",
        "blocked_reason": None,
        "texts": None,
        "images": None,
        "tools": None,
        "stream_holdback_chars": [3, 0, 2, 0, 0, 1],
        "structured_messages": None,
    }
    assert ours["holdback_scalar"] == {
        "action": "NONE",
        "blocked_reason": None,
        "texts": None,
        "images": None,
        "tools": None,
        "stream_holdback_chars": None,
        "structured_messages": None,
    }
    assert ours["tools"] == {
        "action": "NONE",
        "blocked_reason": None,
        "texts": None,
        "images": None,
        "tools": [{"type": "function", "name": "lookup"}],
        "stream_holdback_chars": None,
        "structured_messages": None,
    }


def test_response_default_names() -> None:
    null_action: dict[str, JsonValue] = {"action": None}
    response = generic_guardrail_api.GenericGuardrailAPIResponse.from_dict(null_action)
    assert response.action == "NONE"
    payload: dict[str, JsonValue] = {
        "action": "BLOCKED",
        "blocked_reason": "policy",
        "texts": ["a"],
        "images": [],
    }
    response = generic_guardrail_api.GenericGuardrailAPIResponse.from_dict(payload)
    assert response.action == "BLOCKED"
    assert response.blocked_reason == "policy"
    assert response.texts == ["a"]
    assert response.images == []


def test_response_init_names() -> None:
    param = generic_guardrail_api.GuardrailToolParam(type="function")
    messages = generic_guardrail_api.structured_messages_from_response(
        [{"role": "assistant", "content": None}],
    )
    assert messages is not None
    response = generic_guardrail_api.GenericGuardrailAPIResponse(
        "GUARDRAIL_INTERVENED",
        ["redacted"],
        "policy",
        ["https://img.example/one.png"],
        [param],
        [2],
        messages,
    )
    assert response.action == "GUARDRAIL_INTERVENED"
    assert response.texts == ["redacted"]
    assert response.blocked_reason == "policy"
    assert response.images == ["https://img.example/one.png"]
    assert response.tools == [param]
    assert response.stream_holdback_chars == [2]
    assert response.structured_messages is messages
