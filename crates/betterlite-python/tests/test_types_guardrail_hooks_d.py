import subprocess
import sys
from typing import Final, TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter, ValidationError

from betterlite.types.proxy.guardrails.guardrail_hooks import (
    panw_prisma_airs,
    pillar,
    presidio,
    prompt_security,
    promptguard,
    qohash,
    qualifire,
    repelloai,
    singulr,
    straiker,
)

MODULE_NAMES: Final = (
    "panw_prisma_airs",
    "pillar",
    "presidio",
    "promptguard",
    "prompt_security",
    "qohash",
    "qualifire",
    "repelloai",
    "singulr",
    "straiker",
)
# Recorded from the pinned LiteLLM files, one entry per module
CLASS_COUNTS: Final = {
    "panw_prisma_airs": 1,
    "pillar": 2,
    "presidio": 2,
    "promptguard": 1,
    "prompt_security": 1,
    "qohash": 1,
    "qualifire": 1,
    "repelloai": 5,
    "singulr": 8,
    "straiker": 11,
}
ALIAS_NAMES: Final = {
    "straiker": (
        "StraikerWebhookEventType",
        "StraikerWebhookStreamPhase",
        "StraikerWebhookAction",
    ),
}
CONSTANT_NAMES: Final = {"straiker": ("STRAIKER_WEBHOOK_SCHEMA_VERSION",)}
DUMP_COUNT: Final = 41
REJECTION_COUNT: Final = 25
UI_COUNT: Final = 9
ABSTRACT_COUNT: Final = 27
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
    "panw_prisma_airs",
    "pillar",
    "presidio",
    "promptguard",
    "prompt_security",
    "qohash",
    "qualifire",
    "repelloai",
    "singulr",
    "straiker",
)
ALIASES = {
    "straiker": (
        "StraikerWebhookEventType",
        "StraikerWebhookStreamPhase",
        "StraikerWebhookAction",
    ),
}
CONSTANTS = {"straiker": ("STRAIKER_WEBHOOK_SCHEMA_VERSION",)}
CONSTRAINT_KEYS = (
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "minLength",
    "maxLength",
    "pattern",
    "multipleOf",
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
    ("panw.default", "panw_prisma_airs", "PanwPrismaAirsGuardrailConfigModel", {}),
    (
        "panw.configured",
        "panw_prisma_airs",
        "PanwPrismaAirsGuardrailConfigModel",
        {
            "api_key": "key", "api_base": "https://panw.example", "profile_name": "profile",
            "app_name": "Gateway", "mask_on_block": True, "mask_request_content": True,
            "mask_response_content": True, "fallback_on_error": "allow", "timeout": 60.0,
            "experimental_use_latest_role_message_only": False,
        },
    ),
    ("pillar.params.default", "pillar", "PillarGuardrailConfigModelOptionalParams", {}),
    (
        "pillar.params.configured",
        "pillar",
        "PillarGuardrailConfigModelOptionalParams",
        {
            "on_flagged_action": "block", "async_mode": True, "persist_session": False,
            "include_scanners": False, "include_evidence": False,
            "fallback_on_error": "block", "timeout": 5.0,
        },
    ),
    (
        "pillar.config.configured",
        "pillar",
        "PillarGuardrailConfigModel",
        {
            "api_key": "key", "api_base": "https://pillar.example",
            "optional_params": {"async_mode": True, "timeout": 2.5},
        },
    ),
    ("promptguard.default", "promptguard", "PromptGuardConfigModel", {}),
    (
        "promptguard.configured",
        "promptguard",
        "PromptGuardConfigModel",
        {"api_key": "key", "api_base": "https://promptguard.example", "block_on_error": False},
    ),
    ("prompt_security.default", "prompt_security", "PromptSecurityGuardrailConfigModel", {}),
    (
        "prompt_security.configured",
        "prompt_security",
        "PromptSecurityGuardrailConfigModel",
        {
            "api_key": "key", "api_base": "https://prompt.example",
            "file_sanitization_fail_open": False, "block_on_file_modify": False,
            "streaming_transform_mode": "incremental_diff",
        },
    ),
    ("qohash.default", "qohash", "QostodianNexusConfigModel", {}),
    (
        "qohash.configured",
        "qohash",
        "QostodianNexusConfigModel",
        {"api_base": "http://nexus:8800"},
    ),
    ("qualifire.default", "qualifire", "QualifireGuardrailConfigModel", {}),
    (
        "qualifire.configured",
        "qualifire",
        "QualifireGuardrailConfigModel",
        {
            "api_key": "key", "api_base": "https://qualifire.example", "evaluation_id": "eval",
            "prompt_injections": True, "hallucinations_check": False, "grounding_check": True,
            "pii_check": False, "content_moderation_check": True,
            "tool_selection_quality_check": False, "assertions": ["first", "second"],
            "on_flagged": "monitor",
        },
    ),
    ("repelloai.default", "repelloai", "RepelloAIGuardrailConfigModel", {}),
    (
        "repelloai.configured",
        "repelloai",
        "RepelloAIGuardrailConfigModel",
        {
            "api_key": "key", "api_base": "https://repello.example", "asset_id": "asset",
            "unreachable_fallback": "fail_open",
        },
    ),
    ("singulr.content_block.default", "singulr", "ContentBlock", {}),
    (
        "singulr.tool_call_function.configured",
        "singulr",
        "ToolCallFunction",
        {"name": "lookup", "arguments": "{}"},
    ),
    (
        "singulr.tool_call.configured",
        "singulr",
        "ToolCall",
        {"id": "call_1", "function": {"name": "lookup", "arguments": "{}"}},
    ),
    ("singulr.assistant_message.default", "singulr", "AssistantMessage", {}),
    (
        "singulr.assistant_message.configured",
        "singulr",
        "AssistantMessage",
        {
            "content": [{"type": "text", "text": "hello"}],
            "tool_calls": [{"id": "call_1", "function": {"name": "lookup", "arguments": "{}"}}],
        },
    ),
    (
        "singulr.payload.configured",
        "singulr",
        "SingulrGuardrailPayload",
        {
            "correlation_id": "corr", "model_name": "gpt-4o", "model_provider_name": "openai",
            "guardrail_scope": "scope",
            "messages": [{"role": "user", "content": "hello"}],
            "images": ["https://img.example/one.png"], "tools": [{"type": "function"}],
            "response": {"role": "assistant", "content": "hi"}, "metadata": {"team": "red"},
        },
    ),
    (
        "singulr.mcp_payload.configured",
        "singulr",
        "SingulrMcpGuardrailPayload",
        {
            "model_name": "gpt-4o", "guardrail_scope": "scope", "tool_name": "lookup",
            "tool_arguments": {"query": "weather"}, "mcp_server_name": "server",
            "tool_result": ["sunny"], "metadata": {"team": "red"},
        },
    ),
    ("singulr.response.default", "singulr", "SingulrGuardrailResponse", {}),
    (
        "singulr.response.configured",
        "singulr",
        "SingulrGuardrailResponse",
        {"should_block": True, "blocking_due_to": "policy hit"},
    ),
    (
        "singulr.config.configured",
        "singulr",
        "SingulrGuardrailConfigModel",
        {
            "singulr_api_key": "key", "singulr_api_base": "https://singulr.example",
            "singulr_application_id": "app", "singulr_guardrail_id": "guardrail",
            "block_on_error": True,
        },
    ),
    ("straiker.stream.default", "straiker", "StraikerWebhookStream", {}),
    (
        "straiker.stream.configured",
        "straiker",
        "StraikerWebhookStream",
        {"phase": "assembled", "index": 2},
    ),
    (
        "straiker.event.configured",
        "straiker",
        "StraikerWebhookEvent",
        {"type": "post_call", "id": "evt-1", "stream": {"phase": "assembled", "index": 1}},
    ),
    ("straiker.content.default", "straiker", "StraikerWebhookContent", {}),
    (
        "straiker.content.configured",
        "straiker",
        "StraikerWebhookContent",
        {
            "texts": ["hello"], "images": ["https://img.example/one.png"],
            "structured_messages": [{"role": "user", "content": "hello"}],
            "tools": [{"type": "function"}], "tool_calls": [{"id": "call_1"}],
            "finish_reason": "stop",
        },
    ),
    (
        "straiker.usage.configured",
        "straiker",
        "StraikerWebhookUsage",
        {"input_tokens": 3, "output_tokens": 4},
    ),
    (
        "straiker.context.configured",
        "straiker",
        "StraikerWebhookContext",
        {
            "call_surface": "chat", "mode": ["sync"], "model": "gpt-4o",
            "model_provider": "openai", "destination": "api.openai.com", "session_id": "session",
            "litellm_call_id": "call", "litellm_trace_id": "trace",
            "litellm_version": "1.105.1",
        },
    ),
    (
        "straiker.identity.configured",
        "straiker",
        "StraikerWebhookIdentity",
        {
            "litellm_key": "hashed-key", "litellm_team": "team", "litellm_user_id": "user",
            "litellm_user_email": "user@example.com", "litellm_org_id": "org",
            "end_user_id": "end-user",
        },
    ),
    (
        "straiker.application.configured",
        "straiker",
        "StraikerWebhookApplication",
        {"source": "sdk", "name": "app"},
    ),
    (
        "straiker.request.full",
        "straiker",
        "StraikerWebhookRequest",
        {
            "event": {"type": "pre_call", "id": "evt-1", "stream": {"phase": "none"}},
            "request": {
                "texts": ["hello"],
                "structured_messages": [{"role": "user", "content": "hello"}],
            },
            "response": {
                "action": "GUARDRAIL_INTERVENED", "blocked_reason": "policy",
                "blocked_by": ["policy"], "texts": ["redacted"], "turnId": "turn-1",
            },
            "context": {"call_surface": "chat", "model": "gpt-4o"},
            "identity": {"litellm_key": "hashed-key"},
            "application": {"source": "sdk", "name": "app"},
            "usage": {"input_tokens": 3, "output_tokens": 4},
            "metadata": {"team": "red"},
        },
    ),
    ("straiker.response.default", "straiker", "StraikerWebhookResponse", {}),
    (
        "straiker.response.configured",
        "straiker",
        "StraikerWebhookResponse",
        {
            "action": "BLOCKED", "blocked_reason": "policy",
            "blocked_by": ["policy", "tone"], "texts": ["redacted"],
            "schema_version": "1", "turnId": "turn-1",
        },
    ),
    (
        "straiker.optional_params.default",
        "straiker",
        "StraikerGuardrailConfigModelOptionalParams",
        {},
    ),
    (
        "straiker.optional_params.configured",
        "straiker",
        "StraikerGuardrailConfigModelOptionalParams",
        {
            "timeout": 10.0, "max_retries": 3, "initial_backoff": 0.2, "max_backoff": 4.0,
            "unreachable_fallback": "fail_open", "fail_on_error": False,
            "max_payload_bytes": 1024, "api_version": "v3", "agent_ref": "agent",
            "client": "client", "format_hint": "openai.chat",
            "custom_headers": {"X-Custom": "one"}, "metadata": {"team": "red"}, "verbose": True,
        },
    ),
    (
        "straiker.config.default",
        "straiker",
        "StraikerGuardrailConfigModel",
        {"api_key": "key"},
    ),
    (
        "straiker.config.configured",
        "straiker",
        "StraikerGuardrailConfigModel",
        {
            "api_key": "sk_agt_key", "api_base": "https://straiker.example",
            "default_app": "Gateway", "optional_params": {"timeout": 10.0, "api_version": "v3"},
        },
    ),
]
REJECTIONS = [
    (
        "panw.timeout_low",
        "panw_prisma_airs",
        "PanwPrismaAirsGuardrailConfigModel",
        {"timeout": 0.5},
    ),
    (
        "panw.timeout_high",
        "panw_prisma_airs",
        "PanwPrismaAirsGuardrailConfigModel",
        {"timeout": 61.0},
    ),
    (
        "panw.fallback",
        "panw_prisma_airs",
        "PanwPrismaAirsGuardrailConfigModel",
        {"fallback_on_error": "monitor"},
    ),
    (
        "pillar.on_flagged_action",
        "pillar",
        "PillarGuardrailConfigModelOptionalParams",
        {"on_flagged_action": 5},
    ),
    (
        "prompt_security.streaming",
        "prompt_security",
        "PromptSecurityGuardrailConfigModel",
        {"streaming_transform_mode": "block"},
    ),
    (
        "qualifire.on_flagged",
        "qualifire",
        "QualifireGuardrailConfigModel",
        {"on_flagged": "allow"},
    ),
    (
        "repelloai.unreachable",
        "repelloai",
        "RepelloAIGuardrailConfigModel",
        {"unreachable_fallback": "fail"},
    ),
    ("singulr.role", "singulr", "AssistantMessage", {"role": "user"}),
    ("singulr.tool_missing", "singulr", "ToolCallFunction", {"arguments": "{}"}),
    (
        "straiker.timeout_zero",
        "straiker",
        "StraikerGuardrailConfigModelOptionalParams",
        {"timeout": 0.0},
    ),
    (
        "straiker.retries_negative",
        "straiker",
        "StraikerGuardrailConfigModelOptionalParams",
        {"max_retries": -1},
    ),
    (
        "straiker.initial_backoff_negative",
        "straiker",
        "StraikerGuardrailConfigModelOptionalParams",
        {"initial_backoff": -0.1},
    ),
    (
        "straiker.max_backoff_negative",
        "straiker",
        "StraikerGuardrailConfigModelOptionalParams",
        {"max_backoff": -1.0},
    ),
    (
        "straiker.payload_zero",
        "straiker",
        "StraikerGuardrailConfigModelOptionalParams",
        {"max_payload_bytes": 0},
    ),
    (
        "straiker.api_version",
        "straiker",
        "StraikerGuardrailConfigModelOptionalParams",
        {"api_version": "v2"},
    ),
    (
        "straiker.format_hint",
        "straiker",
        "StraikerGuardrailConfigModelOptionalParams",
        {"format_hint": "claude"},
    ),
    (
        "straiker.unreachable",
        "straiker",
        "StraikerGuardrailConfigModelOptionalParams",
        {"unreachable_fallback": "allow"},
    ),
    ("straiker.api_key_empty", "straiker", "StraikerGuardrailConfigModel", {"api_key": ""}),
    ("straiker.api_key_missing", "straiker", "StraikerGuardrailConfigModel", {}),
    (
        "straiker.event_type",
        "straiker",
        "StraikerWebhookEvent",
        {"type": "during_call", "id": "evt-1"},
    ),
    ("straiker.phase", "straiker", "StraikerWebhookStream", {"phase": "done"}),
    ("straiker.action", "straiker", "StraikerWebhookResponse", {"action": "ALLOW"}),
    ("straiker.context_missing", "straiker", "StraikerWebhookContext", {}),
    ("straiker.application_missing", "straiker", "StraikerWebhookApplication", {}),
    (
        "straiker.event_missing",
        "straiker",
        "StraikerWebhookRequest",
        {
            "request": {}, "context": {"call_surface": "chat"}, "identity": {},
            "application": {"source": "sdk"},
        },
    ),
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
    for name in names:
        value = vars(module)[name]
        if is_typeddict(value):
            schema = TypeAdapter(value).json_schema()
            order[name] = list(value.__annotations__)
        elif isinstance(value, type) and issubclass(value, BaseModel):
            schema = value.model_json_schema()
            order[name] = list(value.model_fields)
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
        "order": order,
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
    order: dict[str, list[str]]
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
    assert ours["types"] == theirs["types"], module_name
    assert ours["order"] == theirs["order"], module_name
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


def test_dump_names(report: FullReport) -> None:
    assert len(report["litellm"]["dumps"]) == DUMP_COUNT
    assert report["betterlite"]["dumps"] == report["litellm"]["dumps"]
    straiker_dumps = report["betterlite"]["dumps"]
    assert '"schema_version": "1"' in straiker_dumps["straiker.request.full"]
    assert '"turn_id": "turn-1"' in straiker_dumps["straiker.response.configured"]
    assert '"blocked_by": ["policy", "tone"]' in straiker_dumps["straiker.response.configured"]


def test_rejection_names(report: FullReport) -> None:
    theirs = report["litellm"]["rejections"]
    ours = report["betterlite"]["rejections"]
    assert len(ours) == REJECTION_COUNT
    assert ours == theirs
    assert ours["panw.timeout_high"] == [
        ["less_than_equal", "Input should be less than or equal to 60", "('timeout',)"],
    ]
    assert ours["straiker.api_key_empty"] == [
        ["string_too_short", "String should have at least 1 character", "('api_key',)"],
    ]
    assert ours["straiker.api_key_missing"] == [["missing", "Field required", "('api_key',)"]]


def test_ui_names(report: FullReport) -> None:
    theirs = report["litellm"]["ui"]
    ours = report["betterlite"]["ui"]
    assert len(theirs) == UI_COUNT
    assert ours == theirs
    assert ours["panw_prisma_airs.PanwPrismaAirsGuardrailConfigModel"] == "PANW Prisma AIRS"
    assert ours["qohash.QostodianNexusConfigModel"] == "Qostodian Nexus"


def test_ui_extra_names(report: FullReport) -> None:
    modules = report["betterlite"]["modules"]
    assert modules["straiker"]["ui_extras"] == {
        "StraikerWebhookStream": {},
        "StraikerWebhookEvent": {},
        "StraikerWebhookContent": {},
        "StraikerWebhookUsage": {},
        "StraikerWebhookContext": {},
        "StraikerWebhookIdentity": {},
        "StraikerWebhookApplication": {},
        "StraikerWebhookRequest": {},
        "StraikerWebhookResponse": {},
        "StraikerGuardrailConfigModelOptionalParams": {},
        "StraikerGuardrailConfigModel": {"api_key": {"secret": True}},
    }
    assert modules["panw_prisma_airs"]["ui_extras"] == {
        "PanwPrismaAirsGuardrailConfigModel": {},
    }


def test_abstract_names(report: FullReport) -> None:
    theirs = report["litellm"]["abstract"]
    ours = report["betterlite"]["abstract"]
    assert len(theirs) == ABSTRACT_COUNT
    assert ours == theirs
    assert ours["panw_prisma_airs.PanwPrismaAirsGuardrailConfigModel"] is False
    assert ours["straiker.StraikerGuardrailConfigModel"] is False


def test_ui_friendly_names() -> None:
    assert (
        panw_prisma_airs.PanwPrismaAirsGuardrailConfigModel.ui_friendly_name()
        == "PANW Prisma AIRS"
    )
    assert pillar.PillarGuardrailConfigModel.ui_friendly_name() == "Pillar Guardrail"
    assert promptguard.PromptGuardConfigModel.ui_friendly_name() == "PromptGuard"
    assert (
        prompt_security.PromptSecurityGuardrailConfigModel.ui_friendly_name()
        == "Prompt Security"
    )
    assert qohash.QostodianNexusConfigModel.ui_friendly_name() == "Qostodian Nexus"
    assert qualifire.QualifireGuardrailConfigModel.ui_friendly_name() == "Qualifire"
    assert repelloai.RepelloAIGuardrailConfigModel.ui_friendly_name() == "RepelloAI Argus"
    assert singulr.SingulrGuardrailConfigModel.ui_friendly_name() == "Singulr"
    assert straiker.StraikerGuardrailConfigModel.ui_friendly_name() == "Straiker"


def test_default_names() -> None:
    assert straiker.STRAIKER_WEBHOOK_SCHEMA_VERSION == "1"
    optional_params = straiker.StraikerGuardrailConfigModelOptionalParams()
    assert optional_params.timeout == 5.0
    assert optional_params.max_retries == 2
    assert optional_params.initial_backoff == 0.1
    assert optional_params.max_backoff == 2.0
    assert optional_params.unreachable_fallback == "fail_closed"
    assert optional_params.fail_on_error is True
    assert optional_params.max_payload_bytes == 524288
    assert optional_params.api_version is None
    assert optional_params.format_hint is None
    assert optional_params.verbose is False
    config = straiker.StraikerGuardrailConfigModel(api_key="key")
    assert config.api_base == "https://api.prod.straiker.ai"
    assert config.default_app == "LiteLLM Gateway"
    assert pillar.PillarGuardrailConfigModelOptionalParams().on_flagged_action == "monitor"
    assert prompt_security.PromptSecurityGuardrailConfigModel().block_on_file_modify is True
    assert qualifire.QualifireGuardrailConfigModel().on_flagged == "block"


def test_required_names() -> None:
    field = straiker.StraikerGuardrailConfigModel.model_fields["api_key"]
    assert field.is_required()
    with pytest.raises(ValidationError):
        straiker.StraikerGuardrailConfigModel.model_validate({})
    context_field = straiker.StraikerWebhookContext.model_fields["call_surface"]
    assert context_field.is_required()


def test_alias_names() -> None:
    response = straiker.StraikerWebhookResponse.model_validate({"turnId": "turn-1"})
    assert response.turn_id == "turn-1"
    by_alias: dict[str, JsonValue] = response.model_dump(by_alias=True)
    assert by_alias["turnId"] == "turn-1"
    plain: dict[str, JsonValue] = response.model_dump()
    assert plain["turn_id"] == "turn-1"


def test_extra_names() -> None:
    response = straiker.StraikerWebhookResponse.model_validate({"unknown_knob": True})
    dumped_response: dict[str, JsonValue] = response.model_dump()
    assert dumped_response["unknown_knob"] is True
    assert dumped_response["blocked_by"] == ()
    content = straiker.StraikerWebhookContent.model_validate({"unknown_knob": True})
    dumped_content: dict[str, JsonValue] = content.model_dump()
    assert "unknown_knob" not in dumped_content
    assert dumped_content["texts"] == []


def test_straiker_event_names() -> None:
    request = straiker.StraikerWebhookRequest(
        event=straiker.StraikerWebhookEvent(type="pre_call", id="evt-1"),
        request=straiker.StraikerWebhookContent(texts=["hello"]),
        context=straiker.StraikerWebhookContext(call_surface="chat"),
        identity=straiker.StraikerWebhookIdentity(),
        application=straiker.StraikerWebhookApplication(source="sdk"),
    )
    assert request.schema_version == "1"
    assert request.event.stream.phase == "none"
    assert request.event.stream.index is None
    assert request.request.texts == ["hello"]
    assert request.response is None
    assert request.identity.litellm_key is None
    dumped: dict[str, JsonValue] = request.model_dump()
    assert dumped["schema_version"] == "1"
    assert dumped["response"] is None


def test_presidio_names() -> None:
    request = presidio.PresidioAnalyzeRequest(
        text="hello", language="en", entities=["PERSON", "US_SSN"]
    )
    assert request["entities"] == ["PERSON", "US_SSN"]
    item = presidio.PresidioAnalyzeResponseItem(entity_type="PERSON", start=0, end=5)
    assert item["entity_type"] == "PERSON"
