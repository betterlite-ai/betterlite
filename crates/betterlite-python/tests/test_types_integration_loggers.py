import subprocess
import sys
from typing import Final, TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter

MODULE_NAMES: Final = (
    "argilla",
    "arize",
    "arize_phoenix",
    "azure_sentinel",
    "base_health_check",
    "cloudzero",
    "custom_logger",
    "datadog",
    "datadog_cost_management",
    "datadog_llm_obs",
    "datadog_metrics",
    "gcs_bucket",
    "langfuse",
    "langfuse_otel",
    "langsmith",
    "newrelic",
    "pagerduty",
    "pointfive",
    "posthog",
    "s3_v2",
    "weave_otel",
    "zerobus",
)
# LiteLLM declares this in a TYPE_CHECKING branch, so its runtime field is Any
PROTOCOL_MODULES: Final = ("arize", "arize_phoenix", "langfuse_otel")
PROTOCOL_LITERALS: Final = ("otlp_grpc", "otlp_http")
EXTRA_CLASS_NAMES: Final = {"pagerduty": frozenset({"StandardLoggingUserAPIKeyMetadata"})}
# Recorded from the pinned LiteLLM files, one entry per module
CLASS_COUNTS: Final = {
    "argilla": 3,
    "arize": 1,
    "arize_phoenix": 1,
    "azure_sentinel": 1,
    "base_health_check": 1,
    "cloudzero": 1,
    "custom_logger": 4,
    "datadog": 5,
    "datadog_cost_management": 2,
    "datadog_llm_obs": 15,
    "datadog_metrics": 3,
    "gcs_bucket": 2,
    "langfuse": 3,
    "langfuse_otel": 2,
    "langsmith": 5,
    "newrelic": 8,
    "pagerduty": 6,
    "pointfive": 3,
    "posthog": 2,
    "s3_v2": 1,
    "weave_otel": 2,
    "zerobus": 4,
}
CONSTANT_COUNTS: Final = {
    "argilla": 1,
    "arize": 0,
    "arize_phoenix": 0,
    "azure_sentinel": 1,
    "base_health_check": 0,
    "cloudzero": 0,
    "custom_logger": 8,
    "datadog": 2,
    "datadog_cost_management": 0,
    "datadog_llm_obs": 0,
    "datadog_metrics": 0,
    "gcs_bucket": 3,
    "langfuse": 0,
    "langfuse_otel": 0,
    "langsmith": 0,
    "newrelic": 14,
    "pagerduty": 0,
    "pointfive": 2,
    "posthog": 1,
    "s3_v2": 0,
    "weave_otel": 0,
    "zerobus": 2,
}
ALIAS_NAMES_BY_MODULE: Final = {
    "cloudzero": frozenset({"CBFRecordDict"}),
    "newrelic": frozenset({"NewRelicMetric"}),
    "s3_v2": frozenset({"S3PartitionGranularity"}),
}
FUNCTION_NAMES_BY_MODULE: Final = {
    "custom_logger": frozenset({"converted_stream_requested", "is_interception_internal_key"}),
}
DESCRIBE_SCRIPT = """
import dataclasses
import importlib
import inspect
import json
import os
from enum import Enum
from types import MappingProxyType
from typing import get_args

os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"

from pydantic import BaseModel, TypeAdapter
from typing_extensions import is_typeddict

MODULE_NAMES = (
    "argilla",
    "arize",
    "arize_phoenix",
    "azure_sentinel",
    "base_health_check",
    "cloudzero",
    "custom_logger",
    "datadog",
    "datadog_cost_management",
    "datadog_llm_obs",
    "datadog_metrics",
    "gcs_bucket",
    "langfuse",
    "langfuse_otel",
    "langsmith",
    "newrelic",
    "pagerduty",
    "pointfive",
    "posthog",
    "s3_v2",
    "weave_otel",
    "zerobus",
)
ALIAS_NAMES = {
    "cloudzero": ("CBFRecordDict",),
    "newrelic": ("NewRelicMetric",),
    "s3_v2": ("S3PartitionGranularity",),
}
# Betterlite carries the Literal that LiteLLM only declares under TYPE_CHECKING
NARROWED_FIELDS = {
    "arize": ("ArizeConfig", "protocol"),
    "arize_phoenix": ("ArizePhoenixConfig", "protocol"),
    "langfuse_otel": ("LangfuseOtelConfig", "protocol"),
}
CONSTANT_KINDS = (
    str,
    int,
    float,
    bool,
    type(None),
    list,
    tuple,
    frozenset,
    set,
    dict,
    MappingProxyType,
)


def missing(value):
    return value is dataclasses.MISSING or value is inspect.Parameter.empty


def value_view(value):
    if missing(value):
        return {"kind": "required"}
    if isinstance(value, MappingProxyType):
        items = [[key, value_view(item)] for key, item in value.items()]
        return {"kind": "mapping", "items": sorted(items)}
    if isinstance(value, (list, tuple)):
        return {"kind": "sequence", "items": [value_view(item) for item in value]}
    if isinstance(value, (frozenset, set)):
        rendered = (json.dumps(value_view(item), sort_keys=True) for item in value)
        return {"kind": "set", "items": sorted(rendered)}
    return {"kind": "scalar", "repr": repr(value)}


def literal_values(node):
    found = set()
    if isinstance(node, dict):
        if "const" in node:
            found.add(json.dumps(node["const"], sort_keys=True))
        for item in node.get("enum", []):
            found.add(json.dumps(item, sort_keys=True))
        for item in node.values():
            found.update(literal_values(item))
    elif isinstance(node, list):
        for item in node:
            found.update(literal_values(item))
    return sorted(found)


def schema_view(schema):
    properties = schema.get("properties", {})
    return {
        "keys": sorted(properties),
        "required": sorted(schema.get("required", [])),
        "fields": {
            name: {
                "default": json.dumps(field.get("default", "no-default"), sort_keys=True),
                "literals": literal_values(field),
            }
            for name, field in properties.items()
        },
    }


def alias_view(schema):
    view = schema_view(schema)
    view["literals"] = literal_values(schema)
    return view


def dataclass_view(cls):
    parameters = cls.__dataclass_params__
    return {
        "fields": [
            {"name": field.name, "default": value_view(field.default), "repr": field.repr}
            for field in dataclasses.fields(cls)
        ],
        "frozen": parameters.frozen,
        "slots": hasattr(cls, "__slots__"),
    }


def class_view(cls):
    if is_typeddict(cls):
        return {
            "kind": "typeddict",
            "schema": schema_view(TypeAdapter(cls).json_schema()),
            "required_keys": sorted(cls.__required_keys__),
            "optional_keys": sorted(cls.__optional_keys__),
        }
    if issubclass(cls, Enum):
        return {
            "kind": "enum",
            "members": [[member.name, member.value] for member in cls],
            "text": issubclass(cls, str),
        }
    if dataclasses.is_dataclass(cls):
        return {"kind": "dataclass", **dataclass_view(cls)}
    if issubclass(cls, BaseModel):
        return {"kind": "model", "schema": schema_view(cls.model_json_schema())}
    if issubclass(cls, tuple) and hasattr(cls, "_fields"):
        return {
            "kind": "namedtuple",
            "fields": list(cls._fields),
            "defaults": {name: value_view(value) for name, value in cls._field_defaults.items()},
        }
    return {"kind": "class", "bases": [base.__name__ for base in cls.__bases__]}


def function_view(function):
    return [
        [parameter.name, value_view(parameter.default)]
        for parameter in inspect.signature(function).parameters.values()
    ]


def custom_logger_behavior(module):
    key_fn = module.is_interception_internal_key
    stream_fn = module.converted_stream_requested
    keys = [
        "_websearch_interception_x",
        "_compression_interception",
        "_headroom_interception",
        "_code_interpreter_interception",
        "normal",
        "",
    ]
    params = [
        {},
        {"_headroom_interception_converted_stream": True},
        {"_headroom_interception_converted_stream": 0},
        {"_websearch_interception_converted_stream": "x"},
        {"other": 1},
    ]
    return {
        "keys": [key_fn(key) for key in keys],
        "custom": [key_fn("x_y", frozenset({"x_"}))],
        "streams": [stream_fn(param) for param in params],
    }


def newrelic_behavior(module):
    record = module.NewRelicMetricRecord(
        team_id="team",
        team_alias="alias",
        model_group="group",
        model="model",
        custom_llm_provider="provider",
        status="success",
        response_cost=0.5,
        prompt_tokens=1,
        completion_tokens=2,
        total_tokens=3,
        duration_ms=4.5,
    )
    return {
        "bucket": list(record.bucket_key),
        "defaults": [record.team_max_budget, record.team_spend],
    }


def zerobus_behavior(module):
    connection = module.ZerobusConnection(
        workspace_url="https://workspace.example",
        workspace_id="workspace",
        server_endpoint="https://server.example",
        client_id="client",
        client_secret="top-secret",
        table_name="catalog.schema.table",
    )
    token = module.ZerobusAccessToken(value="token-value", expires_at=1.5)
    return {
        "connection": repr(connection),
        "token": repr(token),
        "secret_shown": "top-secret" in repr(connection) or "token-value" in repr(token),
    }


def behavior_of(name, module):
    if name == "custom_logger":
        return custom_logger_behavior(module)
    if name == "newrelic":
        return newrelic_behavior(module)
    if name == "zerobus":
        return zerobus_behavior(module)
    return {}


def module_report(module):
    classes = {}
    functions = {}
    constants = {}
    for name, value in vars(module).items():
        if name.startswith("_"):
            continue
        if isinstance(value, type) and getattr(value, "__module__", None) == module.__name__:
            classes[name] = class_view(value)
        elif inspect.isfunction(value) and value.__module__ == module.__name__:
            functions[name] = function_view(value)
        elif type(value) in CONSTANT_KINDS and name != "TYPE_CHECKING":
            constants[name] = value_view(value)
    return {
        "classes": classes,
        "functions": functions,
        "constants": constants,
        "aliases": {},
        "behavior": {},
        "protocol": None,
    }


def strip_narrowed(modules):
    for name, (class_name, field_name) in NARROWED_FIELDS.items():
        fields = modules[name]["classes"][class_name]["schema"]["fields"]
        fields[field_name]["literals"] = []


def side_report(package):
    modules = {}
    for name in MODULE_NAMES:
        module = importlib.import_module(package + "." + name)
        report = module_report(module)
        for alias_name in ALIAS_NAMES.get(name, ()):
            report["aliases"][alias_name] = alias_view(
                TypeAdapter(vars(module)[alias_name]).json_schema()
            )
        report["behavior"] = behavior_of(name, module)
        if name in NARROWED_FIELDS:
            class_name, field_name = NARROWED_FIELDS[name]
            annotation = vars(module)[class_name].__annotations__[field_name]
            report["protocol"] = sorted(get_args(annotation))
        modules[name] = report
    strip_narrowed(modules)
    return {"modules": modules}


print(json.dumps({
    "litellm": side_report("litellm.types.integrations"),
    "betterlite": side_report("betterlite.types.integrations"),
}))
"""


class FieldView(TypedDict):
    default: str
    literals: list[str]


class SchemaView(TypedDict):
    keys: list[str]
    required: list[str]
    fields: dict[str, FieldView]


class AliasView(SchemaView):
    literals: list[str]


class ModuleSide(TypedDict):
    classes: dict[str, dict[str, JsonValue]]
    functions: dict[str, list[list[JsonValue]]]
    constants: dict[str, JsonValue]
    aliases: dict[str, AliasView]
    behavior: dict[str, JsonValue]
    protocol: list[str] | None


class SideReport(TypedDict):
    modules: dict[str, ModuleSide]


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


def test_covered_module_names(report: FullReport) -> None:
    assert tuple(report["litellm"]["modules"]) == MODULE_NAMES
    assert tuple(report["betterlite"]["modules"]) == MODULE_NAMES


@pytest.mark.parametrize("module_name", MODULE_NAMES)
def test_class_names(report: FullReport, module_name: str) -> None:
    theirs = report["litellm"]["modules"][module_name]["classes"]
    ours = report["betterlite"]["modules"][module_name]["classes"]
    extra = EXTRA_CLASS_NAMES.get(module_name, frozenset[str]())
    assert len(theirs) == CLASS_COUNTS[module_name], module_name
    assert len(ours) == CLASS_COUNTS[module_name] + len(extra), module_name
    assert set(ours) - set(theirs) == set(extra), module_name
    assert set(theirs) <= set(ours), module_name
    for name in sorted(theirs):
        assert ours[name] == theirs[name], f"{module_name}.{name}"


@pytest.mark.parametrize("module_name", MODULE_NAMES)
def test_constant_names(report: FullReport, module_name: str) -> None:
    theirs = report["litellm"]["modules"][module_name]["constants"]
    ours = report["betterlite"]["modules"][module_name]["constants"]
    assert len(theirs) == CONSTANT_COUNTS[module_name], module_name
    assert ours == theirs, module_name


@pytest.mark.parametrize("module_name", MODULE_NAMES)
def test_function_names(report: FullReport, module_name: str) -> None:
    theirs = report["litellm"]["modules"][module_name]["functions"]
    ours = report["betterlite"]["modules"][module_name]["functions"]
    expected = FUNCTION_NAMES_BY_MODULE.get(module_name, frozenset[str]())
    assert set(theirs) == set(expected), module_name
    assert ours == theirs, module_name


@pytest.mark.parametrize("module_name", MODULE_NAMES)
def test_alias_names(report: FullReport, module_name: str) -> None:
    theirs = report["litellm"]["modules"][module_name]["aliases"]
    ours = report["betterlite"]["modules"][module_name]["aliases"]
    expected = ALIAS_NAMES_BY_MODULE.get(module_name, frozenset[str]())
    assert set(theirs) == set(expected), module_name
    assert ours == theirs, module_name


@pytest.mark.parametrize("module_name", MODULE_NAMES)
def test_protocol_names(report: FullReport, module_name: str) -> None:
    theirs = report["litellm"]["modules"][module_name]["protocol"]
    ours = report["betterlite"]["modules"][module_name]["protocol"]
    if module_name not in PROTOCOL_MODULES:
        assert theirs is None
        assert ours is None
        return
    assert theirs == []
    assert ours == list(PROTOCOL_LITERALS)


def test_behavior_names(report: FullReport) -> None:
    for module_name in MODULE_NAMES:
        theirs = report["litellm"]["modules"][module_name]["behavior"]
        ours = report["betterlite"]["modules"][module_name]["behavior"]
        assert ours == theirs, module_name
    for module_name in ("custom_logger", "newrelic", "zerobus"):
        assert report["betterlite"]["modules"][module_name]["behavior"]
    zerobus = report["betterlite"]["modules"]["zerobus"]["behavior"]
    assert zerobus["secret_shown"] is False
