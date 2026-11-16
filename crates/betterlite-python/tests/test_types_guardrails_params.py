import subprocess
import sys
from typing import Final, TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter

from betterlite.types.guardrails import endpoints, params
from betterlite.types.proxy.guardrails.guardrail_hooks.decision_model import DecisionModelCheck

DESCRIBE_SCRIPT = """
import json
import os
import sys
from enum import Enum

os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"

import typing_extensions
from pydantic import BaseModel, TypeAdapter, ValidationError

import litellm.types.guardrails as theirs

import betterlite.types.guardrails.endpoints as our_endpoints
import betterlite.types.guardrails.params as our_params


def collect_literals(node, found):
    if isinstance(node, dict):
        if isinstance(node.get("enum"), list):
            found.update(json.dumps(item, sort_keys=True) for item in node["enum"])
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
    if "prefixItems" in node:
        return {"tuple": [type_shape(item) for item in node["prefixItems"]]}
    kind = node.get("type")
    if kind == "object":
        extra = node.get("additionalProperties")
        if extra is None or extra is True:
            return "json"
        inner = type_shape(extra)
        return "json" if inner == "json" else {"object": inner}
    if kind == "array":
        return {"array": type_shape(node.get("items", {})), "unique": node.get("uniqueItems")}
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
        "constraints": {
            name: sorted(
                (key, value) for key, value in field.items() if key.endswith("inimum")
            )
            for name, field in properties.items()
        },
    }


def describe(module, name):
    value = vars(module)[name]
    if issubclass(value, BaseModel):
        described = shape(value.model_json_schema())
        described["bases"] = [
            base.__name__
            for base in value.__bases__
            if base.__name__ not in ("BaseModel", "ResponseObject")
        ]
        return {"kind": "model", **described}
    if issubclass(value, Enum):
        return {"kind": "enum", "members": [[member.name, member.value] for member in value]}
    return {"kind": "typeddict", **shape(TypeAdapter(value).json_schema())}


def defined(module):
    return sorted(
        name
        for name, value in vars(module).items()
        if not name.startswith("_") and getattr(value, "__module__", None) == module.__name__
    )


def dumps(model):
    return json.dumps(model.model_dump(), sort_keys=True, default=str)


def outcome(module, name, kwargs):
    try:
        return {"dump": dumps(vars(module)[name](**kwargs))}
    except ValidationError as error:
        return {"errors": sorted([item["type"], item["msg"]] for item in error.errors())}


def validated(module, name, payload):
    try:
        return {"dump": dumps(vars(module)[name].model_validate(payload))}
    except ValidationError as error:
        return {"errors": sorted([item["type"], item["msg"]] for item in error.errors())}


ours = {**vars(our_params), **vars(our_endpoints)}
report = {"defined": {"betterlite": sorted(set(defined(our_params)) | set(defined(our_endpoints)))}}
names = report["defined"]["betterlite"]
report["shapes"] = {
    "litellm": {name: describe(theirs, name) for name in names},
    "betterlite": {
        name: describe(our_params if name in vars(our_params) else our_endpoints, name)
        for name in names
    },
}
report["litellm_has"] = {name: name in vars(theirs) for name in names}

PARAMS_CASES = json.loads(sys.argv[1])
report["outcomes"] = {"litellm": {}, "betterlite": {}}
for label, name, kwargs in PARAMS_CASES:
    report["outcomes"]["litellm"][label] = outcome(theirs, name, kwargs)
    report["outcomes"]["betterlite"][label] = outcome(
        our_params if name in vars(our_params) else our_endpoints, name, kwargs
    )
VALIDATED_CASES = json.loads(sys.argv[2])
report["validated"] = {"litellm": {}, "betterlite": {}}
for label, name, payload in VALIDATED_CASES:
    report["validated"]["litellm"][label] = validated(theirs, name, payload)
    report["validated"]["betterlite"][label] = validated(
        our_params if name in vars(our_params) else our_endpoints, name, payload
    )


def access(module):
    model = vars(module)["LitellmParams"](guardrail="g", mode="pre_call", unknown_key=3)
    return {
        "contains": ["guardrail" in model, "unknown_key" in model, "absent" in model],
        "get": [
            model.get("guardrail"),
            model.get("unknown_key"),
            model.get("absent"),
            model.get("absent", 7),
        ],
        "item": [model["guardrail"], model["unknown_key"], model["mode"]],
    }


report["access"] = {"litellm": access(theirs), "betterlite": access(our_params)}
print(json.dumps(report))
"""

PARAMS_NAMES: Final = frozenset({
    "BaseLitellmParams",
    "BlockedWord",
    "ContentFilterAction",
    "ContentFilterConfigModel",
    "ContentFilterPattern",
    "DynamicGuardrailParams",
    "GUARDRAIL_DEFINITION_LOCATION",
    "Guardrail",
    "GuardrailEventHooks",
    "LitellmParams",
    "Mode",
    "guardrailConfig",
})
ENDPOINT_NAMES: Final = frozenset({
    "ApplyGuardrailRequest",
    "ApplyGuardrailResponse",
    "GuardrailInfoResponse",
    "GuardrailUIAddGuardrailSettings",
    "ListGuardrailsResponse",
    "PatchGuardrailRequest",
    "PresidioPerRequestConfig",
})
BASE: Final = {"guardrail": "g", "mode": "pre_call"}
INFO: Final = {"guardrail_name": "n"}
CASES: Final = [
    ("action_block", "BlockedWord", {"keyword": "k", "action": "BLOCK"}),
    ("action_bad", "BlockedWord", {"keyword": "k", "action": "block"}),
    ("word_description", "BlockedWord", {"keyword": "k", "action": "MASK", "description": "d"}),
    (
        "pattern_regex",
        "ContentFilterPattern",
        {"pattern_type": "regex", "pattern": "a+", "action": "MASK"},
    ),
    ("pattern_bad_type", "ContentFilterPattern", {"pattern_type": "glob", "action": "MASK"}),
    ("pattern_missing", "ContentFilterPattern", {"pattern_type": "prebuilt"}),
    (
        "filter_config",
        "ContentFilterConfigModel",
        {
            "patterns": [{"pattern_type": "prebuilt", "pattern_name": "us_ssn", "action": "BLOCK"}],
            "blocked_words": [{"keyword": "k", "action": "BLOCK"}],
            "categories": [{"category": "harmful_self_harm", "action": "BLOCK"}],
            "severity_threshold": "high",
        },
    ),
    ("filter_default", "ContentFilterConfigModel", {}),
    ("base_default", "BaseLitellmParams", {}),
    (
        "base_configured",
        "BaseLitellmParams",
        {
            "api_key": "k",
            "category_thresholds": {"jailbreak": 0.5},
            "detect_secrets_config": {"a": 1},
            "additional_provider_specific_params": {"x": [1, {"y": None}]},
            "extra_headers": ["x-request-id"],
            "timeout": 3.5,
            "unknown_key": {"a": 1},
        },
    ),
    ("base_on_violation", "BaseLitellmParams", {"on_violation": "end_session"}),
    ("base_on_violation_bad", "BaseLitellmParams", {"on_violation": "stop"}),
    ("base_fallback_upper", "BaseLitellmParams", {"unreachable_fallback": "FAIL_OPEN"}),
    ("base_fallback_bad", "BaseLitellmParams", {"unreachable_fallback": "retry"}),
    ("base_fallback_null", "BaseLitellmParams", {"unreachable_fallback": None}),
    ("base_sensitive_upper", "BaseLitellmParams", {"on_sensitive_data": "BLOCK"}),
    ("base_route_missing", "BaseLitellmParams", {"on_sensitive_data": "route"}),
    ("base_route_upper_missing", "BaseLitellmParams", {"on_sensitive_data": "ROUTE"}),
    (
        "base_route_set",
        "BaseLitellmParams",
        {"on_sensitive_data": "route", "sensitive_data_route_to_model": "local"},
    ),
    (
        "base_route_empty",
        "BaseLitellmParams",
        {"on_sensitive_data": "route", "sensitive_data_route_to_model": ""},
    ),
    ("base_chars_zero", "BaseLitellmParams", {"max_input_chars": 0}),
    ("base_chars_one", "BaseLitellmParams", {"max_input_chars": 1}),
    ("base_calls_zero", "BaseLitellmParams", {"max_concurrent_decision_calls": 0}),
    ("base_calls_one", "BaseLitellmParams", {"max_concurrent_decision_calls": 1}),
    ("base_timeout_text", "BaseLitellmParams", {"timeout": "5"}),
    (
        "mode_tags",
        "Mode",
        {"tags": {"a": "pre_call", "b": ["post_call", "during_call"]}, "default": "pre_call"},
    ),
    ("mode_missing_tags", "Mode", {"default": "pre_call"}),
    ("params_default", "LitellmParams", BASE),
    ("params_default_on_null", "LitellmParams", {**BASE, "default_on": None}),
    ("params_default_on_true", "LitellmParams", {**BASE, "default_on": True}),
    ("params_default_on_false", "LitellmParams", {**BASE, "default_on": False}),
    ("params_mode_missing", "LitellmParams", {"guardrail": "g"}),
    ("params_guardrail_missing", "LitellmParams", {"mode": "pre_call"}),
    ("params_mode_upper", "LitellmParams", {"guardrail": "g", "mode": "PRE_CALL"}),
    (
        "params_mode_list",
        "LitellmParams",
        {"guardrail": "g", "mode": ["PRE_CALL", "Post_Call", "x"]},
    ),
    ("params_mode_mapping", "LitellmParams", {"guardrail": "g", "mode": {"tags": {"t": "PRE"}}}),
    (
        "params_mode_mapping_default",
        "LitellmParams",
        {"guardrail": "g", "mode": {"tags": {"t": ["pre_call"]}, "default": "Post_Call"}},
    ),
    ("params_mode_number", "LitellmParams", {"guardrail": "g", "mode": 3}),
    ("params_mode_list_number", "LitellmParams", {"guardrail": "g", "mode": ["a", 3]}),
    ("params_timeout_text", "LitellmParams", {**BASE, "timeout": "30"}),
    ("params_timeout_padded", "LitellmParams", {**BASE, "timeout": " 2.5 "}),
    ("params_timeout_empty", "LitellmParams", {**BASE, "timeout": ""}),
    ("params_timeout_null", "LitellmParams", {**BASE, "timeout": None}),
    ("params_timeout_int", "LitellmParams", {**BASE, "timeout": 4}),
    ("params_timeout_float", "LitellmParams", {**BASE, "timeout": 0.25}),
    ("params_timeout_bad", "LitellmParams", {**BASE, "timeout": "soon"}),
    ("params_timeout_list", "LitellmParams", {**BASE, "timeout": [1]}),
    ("params_timeout_dict", "LitellmParams", {**BASE, "timeout": {"a": 1}}),
    ("params_violation_block", "LitellmParams", {**BASE, "on_violation": "block"}),
    ("params_violation_alert", "LitellmParams", {**BASE, "on_violation": "alert"}),
    ("params_violation_warn", "LitellmParams", {**BASE, "on_violation": "warn"}),
    ("params_violation_end", "LitellmParams", {**BASE, "on_violation": "end_session"}),
    (
        "params_violation_security",
        "LitellmParams",
        {"guardrail": "mcp_security", "mode": "pre_mcp_call", "on_violation": "block"},
    ),
    (
        "params_checks_bedrock",
        "LitellmParams",
        {**BASE, "checks": {"contentFilter": {"categories": [{"category": "HATE"}]}}},
    ),
    ("params_checks_empty", "LitellmParams", {**BASE, "checks": {}}),
    (
        "params_checks_decision",
        "LitellmParams",
        {
            "guardrail": "decision_model",
            "mode": "pre_call",
            "decision_model": "m",
            "checks": [{"name": "a", "instructions": "do"}],
        },
    ),
    (
        "params_checks_wrong_guardrail",
        "LitellmParams",
        {**BASE, "checks": [{"name": "a", "instructions": "do"}]},
    ),
    ("params_checks_empty_list", "LitellmParams", {**BASE, "checks": []}),
    (
        "params_checks_empty_list_decision",
        "LitellmParams",
        {"guardrail": "decision_model", "mode": "m", "checks": []},
    ),
    ("params_flagged_inject", "LitellmParams", {**BASE, "on_flagged": "inject_system_message"}),
    ("params_flagged_bad", "LitellmParams", {**BASE, "on_flagged": "warn"}),
    ("params_route_missing", "LitellmParams", {**BASE, "on_sensitive_data": "ROUTE"}),
    (
        "params_bedrock",
        "LitellmParams",
        {**BASE, "guardrailIdentifier": "gr", "guardrailVersion": "DRAFT"},
    ),
    (
        "params_presidio",
        "LitellmParams",
        {**BASE, "presidio_filter_scope": "both", "pii_entities_config": {"PERSON": "MASK"}},
    ),
    (
        "params_cisco",
        "LitellmParams",
        {**BASE, "api_key": "k", "optional_params": {"timeout": 5.0}},
    ),
    ("params_extra", "LitellmParams", {**BASE, "anything": {"a": [1]}}),
    ("info_default", "GuardrailInfoResponse", INFO),
    ("info_location_db", "GuardrailInfoResponse", {**INFO, "guardrail_definition_location": "db"}),
    (
        "info_location_bad",
        "GuardrailInfoResponse",
        {**INFO, "guardrail_definition_location": "file"},
    ),
    (
        "info_full",
        "GuardrailInfoResponse",
        {
            "guardrail_id": "i",
            "guardrail_name": "n",
            "litellm_params": {"api_key": "k", "default_on": True},
            "guardrail_info": {"a": 1},
            "created_at": "2025-01-02T03:04:05",
            "updated_at": "2025-01-02T03:04:05",
        },
    ),
    ("info_missing_name", "GuardrailInfoResponse", {"guardrail_id": "i"}),
    ("list_default", "ListGuardrailsResponse", {"guardrails": [INFO]}),
    ("list_missing", "ListGuardrailsResponse", {}),
    (
        "ui_settings",
        "GuardrailUIAddGuardrailSettings",
        {
            "supported_entities": ["PERSON"],
            "supported_actions": ["BLOCK"],
            "supported_modes": ["pre_call"],
            "supported_modes_by_provider": {"presidio": ["pre_call"]},
            "pii_entity_categories": [{"category": "General", "entities": ["PERSON"]}],
        },
    ),
    (
        "ui_settings_providers",
        "GuardrailUIAddGuardrailSettings",
        {
            "supported_entities": [],
            "supported_actions": [],
            "supported_modes": [],
            "supported_modes_by_provider": {},
            "pii_entity_categories": [],
            "content_filter_settings": {"a": 1},
            "decision_model_providers": ["openai", "azure"],
        },
    ),
    ("ui_settings_missing", "GuardrailUIAddGuardrailSettings", {"supported_entities": []}),
    ("presidio_request_default", "PresidioPerRequestConfig", {}),
    (
        "presidio_request_entities",
        "PresidioPerRequestConfig",
        {"language": "en", "entities": ["PERSON", "UUID"]},
    ),
    ("presidio_request_bad", "PresidioPerRequestConfig", {"entities": ["WIZARD"]}),
    ("apply_request", "ApplyGuardrailRequest", {"guardrail_name": "g", "text": "t"}),
    (
        "apply_request_full",
        "ApplyGuardrailRequest",
        {
            "guardrail_name": "g",
            "text": "t",
            "language": "de",
            "entities": ["EMAIL_ADDRESS"],
            "input_type": "response",
            "messages": [{"role": "user", "content": "c"}],
            "metadata": {"a": 1},
        },
    ),
    ("apply_request_missing", "ApplyGuardrailRequest", {"guardrail_name": "g"}),
    (
        "apply_request_input_null",
        "ApplyGuardrailRequest",
        {"guardrail_name": "g", "text": "t", "input_type": None},
    ),
    ("apply_response", "ApplyGuardrailResponse", {"response_text": "r"}),
    ("apply_response_missing", "ApplyGuardrailResponse", {}),
    ("patch_default", "PatchGuardrailRequest", {}),
    (
        "patch_full",
        "PatchGuardrailRequest",
        {"guardrail_name": "n", "litellm_params": {"api_base": "b"}, "guardrail_info": {"a": 1}},
    ),
]
VALIDATED: Final = [
    ("validate_params_default_on", "LitellmParams", BASE),
    ("validate_params_default_on_true", "LitellmParams", {**BASE, "default_on": True}),
    ("validate_params_mode_upper", "LitellmParams", {"guardrail": "g", "mode": "PRE_CALL"}),
    ("validate_base_default", "BaseLitellmParams", {}),
    ("validate_patch", "PatchGuardrailRequest", {"litellm_params": {"on_sensitive_data": "BLOCK"}}),
]
TIMEOUT_SECONDS: Final = 3.5


class Report(TypedDict):
    defined: dict[str, list[str]]
    shapes: dict[str, dict[str, JsonValue]]
    litellm_has: dict[str, bool]
    outcomes: dict[str, dict[str, JsonValue]]
    validated: dict[str, dict[str, JsonValue]]
    access: dict[str, dict[str, JsonValue]]


@pytest.fixture(scope="module")
def report() -> Report:
    cases = TypeAdapter(JsonValue).dump_json(CASES).decode()
    validated = TypeAdapter(JsonValue).dump_json(VALIDATED).decode()
    completed = subprocess.run(
        [sys.executable, "-c", DESCRIBE_SCRIPT, cases, validated],
        check=True,
        capture_output=True,
        text=True,
    )
    return TypeAdapter(Report).validate_json(completed.stdout)


def test_defined_names(report: Report) -> None:
    assert set(report["defined"]["betterlite"]) == PARAMS_NAMES | ENDPOINT_NAMES
    assert all(report["litellm_has"].values())


@pytest.mark.parametrize("name", sorted(PARAMS_NAMES | ENDPOINT_NAMES))
def test_shape_names(report: Report, name: str) -> None:
    shapes = report["shapes"]
    assert shapes["betterlite"][name] == shapes["litellm"][name]


def test_outcome_names(report: Report) -> None:
    outcomes = report["outcomes"]
    assert outcomes["betterlite"] == outcomes["litellm"]
    assert len(outcomes["litellm"]) == len(CASES)
    assert len({label for label, _name, _kwargs in CASES}) == len(CASES)


def test_validated_names(report: Report) -> None:
    validated = report["validated"]
    assert validated["betterlite"] == validated["litellm"]


def test_access_names(report: Report) -> None:
    access = report["access"]
    assert access["betterlite"] == access["litellm"]
    assert access["betterlite"]["contains"] == [True, True, False]


def test_default_on_names() -> None:
    assert params.LitellmParams(guardrail="g", mode="pre_call").default_on is False
    assert params.LitellmParams(guardrail="g", mode="pre_call", default_on=None).default_on is False
    assert params.LitellmParams(guardrail="g", mode="pre_call", default_on=True).default_on is True
    assert params.LitellmParams.model_validate(BASE).default_on is False


def test_mode_union_names() -> None:
    text = params.LitellmParams(guardrail="g", mode="PRE_CALL")
    assert text.mode == "pre_call"
    listed = params.LitellmParams(guardrail="g", mode=["A", "b"])
    assert listed.mode == ["a", "b"]
    mode = params.Mode(tags={"t": ["pre_call"]}, default="post_call")
    tagged = params.LitellmParams.model_validate({"guardrail": "g", "mode": mode})
    assert tagged.mode == mode
    payload: dict[str, JsonValue] = {"guardrail": "g", "mode": {"tags": {"t": "x"}}}
    from_mapping = params.LitellmParams.model_validate(payload)
    assert from_mapping.mode == params.Mode(tags={"t": "x"})


def test_timeout_names() -> None:
    assert params.LitellmParams(guardrail="g", mode="m", timeout="3.5").timeout == TIMEOUT_SECONDS
    assert params.LitellmParams(guardrail="g", mode="m", timeout="").timeout is None
    assert params.LitellmParams(guardrail="g", mode="m", timeout=2).timeout == float(2)


def test_checks_names() -> None:
    decision = params.LitellmParams(
        guardrail="decision_model", mode="m", checks=[{"name": "a", "instructions": "do"}]
    )
    assert decision.checks == (DecisionModelCheck(name="a", instructions="do"),)
    with pytest.raises(ValueError, match="only supported by guardrail='decision_model'"):
        params.LitellmParams(guardrail="g", mode="m", checks=[{"name": "a", "instructions": "do"}])


def test_enum_names() -> None:
    assert [hook.value for hook in params.GuardrailEventHooks] == [
        "pre_call",
        "post_call",
        "during_call",
        "logging_only",
        "pre_mcp_call",
        "during_mcp_call",
        "post_mcp_call",
        "realtime_input_transcription",
    ]
    info = endpoints.GuardrailInfoResponse(guardrail_name="n")
    assert info.guardrail_definition_location == "config"
    assert params.MCP_SECURITY_ON_VIOLATION == {"block", "alert"}


def test_ui_settings_names() -> None:
    settings = endpoints.GuardrailUIAddGuardrailSettings(
        supported_entities=[],
        supported_actions=[],
        supported_modes=[],
        supported_modes_by_provider={},
        pii_entity_categories=[],
    )
    assert settings.decision_model_providers == ()
    assert settings.content_filter_settings is None
