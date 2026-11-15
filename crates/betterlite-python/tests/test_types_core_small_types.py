import json
import subprocess
import sys
from typing import Final

import pytest
from pydantic import JsonValue, TypeAdapter

LITELLM: Final = "litellm.types"
BETTERLITE: Final = "betterlite.types"
NOT_PORTED: Final = {"wire"}
MIN_WEIGHT_OUTCOMES: Final = 20
MIN_WEIGHT_KINDS: Final = 3
MIN_CONTROL_KINDS: Final = 5

PROBE: Final = """
import dataclasses, enum, importlib, inspect, json, sys, typing
from pydantic import BaseModel, ValidationError
from pydantic_core import to_jsonable_python

prefix, mode, spec = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])


def literals(tp):
    if typing.get_origin(tp) is typing.Literal:
        return [list(typing.get_args(tp))]
    found = []
    for arg in typing.get_args(tp):
        found.extend(literals(arg))
    return found


def named(value):
    return value.__name__ if inspect.isclass(value) else repr(value)


def plain(value):
    return json.loads(json.dumps(to_jsonable_python(value, fallback=named)))


def model_fields(obj):
    fields = {}
    for name, info in obj.model_fields.items():
        entry = {
            "required": info.is_required(),
            "description": info.description,
            "constraints": [repr(item) for item in info.metadata],
            "literals": literals(info.annotation),
        }
        if info.default_factory is not None:
            entry["default"] = plain(info.default_factory())
        elif not info.is_required():
            entry["default"] = plain(info.default)
        fields[name] = entry
    return fields


def dataclass_fields(obj):
    fields = []
    for item in dataclasses.fields(obj):
        missing = item.default is dataclasses.MISSING
        fields.append({
            "name": item.name,
            "required": missing and item.default_factory is dataclasses.MISSING,
            "default": None if missing else plain(item.default),
            "kw_only": item.kw_only,
            "metadata": plain(dict(item.metadata)),
            "literals": literals(item.type),
        })
    return fields


def describe(obj):
    origin = typing.get_origin(obj)
    if hasattr(obj, "__required_keys__"):
        hints = obj.__annotations__
        return {
            "kind": "typeddict",
            "required": sorted(obj.__required_keys__),
            "optional": sorted(obj.__optional_keys__),
            "literals": {key: literals(hints[key]) for key in sorted(hints)},
        }
    if inspect.isclass(obj) and issubclass(obj, enum.Enum):
        members = {name: plain(item.value) for name, item in obj.__members__.items()}
        return {"kind": "enum", "str": issubclass(obj, str), "members": members}
    if inspect.isclass(obj) and issubclass(obj, BaseModel):
        return {
            "kind": "model",
            "bases": [base.__name__ for base in obj.__bases__],
            "config": plain(dict(obj.model_config)),
            "fields": model_fields(obj),
        }
    if inspect.isclass(obj) and dataclasses.is_dataclass(obj):
        return {
            "kind": "dataclass",
            "frozen": obj.__dataclass_params__.frozen,
            "slots": hasattr(obj, "__slots__"),
            "fields": dataclass_fields(obj),
        }
    if origin is typing.Literal:
        return {"kind": "literal", "values": list(typing.get_args(obj))}
    if origin is typing.Annotated:
        base, *extras = typing.get_args(obj)
        kept = [repr(item) for item in extras if type(item).__name__ != "AfterValidator"]
        return {"kind": "annotated", "base": repr(base), "metadata": kept}
    if callable(obj) and not inspect.isclass(obj):
        return {"kind": "callable"}
    return {"kind": "value", "value": plain(obj)}


def outcome(call, value):
    try:
        return repr(call(value))
    except ValidationError as error:
        return repr([(item["type"], item["loc"], item["msg"]) for item in error.errors()])


def load(name):
    return importlib.import_module(prefix + "." + name)


def routing_cases():
    nan, inf = float("nan"), float("inf")
    weights = [
        None, {}, {"a": {}}, {"a": {"x": 1.0}}, {"a": {"x": 0.0, "y": 0.5}}, {"a": {"x": 0.0}},
        {"a": {"x": -1.0}}, {"a": {"x": 1}}, {"a": {"x": "1.0"}}, {"a": {"x": True}},
        {"a": {"x": nan}}, {"a": {"x": inf}}, {"a": {"x": 1.0}, "b": {"y": 0.0}},
        {"a": {"x": 1.0}, "b": {}}, {"": {"x": 1.0}}, {" ": {"x": 1.0}}, {"a": {"": 1.0}},
        {"a": {"\\t": 1.0}}, {"a": 1.0}, {"a": None}, [], "weights", 5, {1: {"x": 1.0}},
    ]
    settings = [{"weights": item} for item in weights] + [
        {}, {"other": 1}, {"weights": None, "other": [1]}, [], None, 5, "x", {1: 2},
    ]
    return weights, settings


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
            if (inspect.isclass(item) or inspect.isfunction(item))
            and item.__module__ == module.__name__
        )
elif mode == "weights":
    module = load("router_weights")
    weights, settings = routing_cases()
    result = {
        "weights": [outcome(module.validate_router_weights, item) for item in weights],
        "settings": [outcome(module.validate_router_settings_dict, item) for item in settings],
    }
elif mode == "control":
    module = load("litellm_params")
    cases = [
        None, 5, 1, 0, -3, "12", "0", "007", " 5", "+5", "-5", "5.0", "", "\\u0663", True, 5.0,
        "1" * 18, "1" * 19, "9" * 18, 10**18, 10**18 - 1, [5], {"a": 1},
    ]
    result = {
        "control": [
            outcome(lambda item: module.ControlOptions(stream_chunk_size=item), case)
            for case in cases
        ],
        "convert": [outcome(module._int_from_decimal_string, case) for case in cases],
    }
elif mode == "payload":
    module = load("services")
    payload = module.ServiceLoggerPayload(
        is_error=False,
        service=module.ServiceTypes.REDIS,
        duration=1.5,
        call_type="completion",
        event_metadata={"k": 1},
    )
    result = {
        "plain": repr(payload.to_json()),
        "no_none": repr(payload.to_json(exclude_none=True)),
    }
print(json.dumps(result))
"""

MODULES: Final[dict[str, list[str]]] = {
    "litellm_params": [
        "RetryStrategy",
        "AgenticSurface",
        "RoutingStrategyName",
        "TRUSTED_CALLBACK_VARS_FIELD",
        "ADDRESSED_RESPONSE_ID_FIELD",
        "WIRE_NAME",
        "ProviderConnection",
        "BedrockBatchConnection",
        "ConnectionSettings",
        "DispatchOptions",
        "RoutingOptions",
        "DeploymentOptions",
        "SpecializedRouterOptions",
        "CachingOptions",
        "CostOptions",
        "ObservabilityOptions",
        "AgenticLoopOptions",
        "GuardrailOptions",
        "PromptOptions",
        "ResponseOptions",
        "MAX_CONTROL_INT_DIGITS",
        "_int_from_decimal_string",
        "ControlOptions",
        "MockOptions",
        "LiteLLMOptions",
        "CallState",
        "AgenticLoopState",
        "RouterState",
        "ProxyRequestState",
        "EntrypointState",
        "InternalState",
        "KWARG_ARTIFACTS",
        "LITELLM_OWNED_ROOTS",
        "wire_names",
        "owned_wire_names",
        "OWNED_KWARG_NAMES",
        "AGENTIC_LOOP_KWARG_NAMES",
        "BEDROCK_BATCH_KWARG_NAMES",
    ],
    "router_weights": [
        "RouterWeightIdentifier",
        "RouterWeight",
        "RouterWeights",
        "_validate_positive_router_weights",
        "validate_router_weights",
        "validate_router_settings_dict",
    ],
    "lar1": ["LAR1Act", "LAR1Time", "LAR1Mind", "LAR1Evidence", "LAR1Metadata"],
    "scheduler": ["DefaultPriorities"],
    "services": [
        "ServiceMetrics",
        "ServiceTypes",
        "ServiceConfig",
        "DEFAULT_SERVICE_CONFIGS",
        "ServiceEventMetadata",
        "ServiceLoggerPayload",
    ],
    "tag_management": [
        "TagBase",
        "TagConfig",
        "TagNewRequest",
        "TagUpdateRequest",
        "TagDeleteRequest",
        "TagInfoRequest",
    ],
    "tool_management": [
        "ToolCallPolicy",
        "ToolInputPolicy",
        "ToolOutputPolicy",
        "ToolDiscoveryUser",
        "LiteLLM_ToolTableRow",
        "ToolListResponse",
        "ToolPolicyUpdateRequest",
        "ToolPolicyUpdateResponse",
        "ToolPolicyOverrideRow",
        "ToolPolicyOption",
        "ToolPolicyOptionsResponse",
        "ToolDetailResponse",
        "ToolUsageLogEntry",
        "ToolUsageLogsResponse",
        "ToolSpendEntry",
        "ToolSpendDailyEntry",
        "ToolSpendResponse",
    ],
    "object_permission": ["ObjectPermissionDict"],
    "caching": [
        "LiteLLMCacheType",
        "SemanticCacheScope",
        "CachingSupportedCallTypes",
        "DEFAULT_CACHING_SUPPORTED_CALL_TYPES",
        "RedisPipelineIncrementOperation",
        "RedisPipelineSetOperation",
        "RedisPipelineRpushOperation",
        "RedisPipelineLpopOperation",
        "DynamicCacheControl",
        "CachePingResponse",
        "HealthCheckCacheParams",
        "EMBEDDING_CACHE_FORMAT_VERSION",
        "CachedEmbedding",
    ],
    "compression": ["CompressedResult"],
}

OBJECT: Final = TypeAdapter(dict[str, JsonValue])
STRINGS: Final = TypeAdapter(list[str])


def run_probe(prefix: str, mode: str, spec: str) -> dict[str, JsonValue]:
    done = subprocess.run(
        [sys.executable, "-c", PROBE, prefix, mode, spec],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    return OBJECT.validate_json(done.stdout)


@pytest.fixture(scope="module")
def described() -> dict[str, dict[str, JsonValue]]:
    names = list(MODULES)
    spec = json.dumps(MODULES)
    return {
        "theirs": run_probe(LITELLM, "describe", spec),
        "ours": run_probe(BETTERLITE, "describe", spec),
        "defined": run_probe(LITELLM, "defined", json.dumps(names)),
    }


@pytest.mark.parametrize("module", MODULES)
def test_names_defined(module: str, described: dict[str, dict[str, JsonValue]]) -> None:
    defined = STRINGS.validate_python(described["defined"][module])
    assert set(defined) <= set(MODULES[module]) | NOT_PORTED


@pytest.mark.parametrize("module", MODULES)
def test_names_match_litellm(module: str, described: dict[str, dict[str, JsonValue]]) -> None:
    theirs = OBJECT.validate_python(described["theirs"][module])
    ours = OBJECT.validate_python(described["ours"][module])
    assert all(item != {"kind": "missing"} for item in theirs.values())
    assert ours == theirs


def test_names_weights_match() -> None:
    theirs = run_probe(LITELLM, "weights", "{}")
    assert run_probe(BETTERLITE, "weights", "{}") == theirs
    outcomes = STRINGS.validate_python(theirs["weights"])
    assert len(outcomes) > MIN_WEIGHT_OUTCOMES
    assert len(set(outcomes)) > MIN_WEIGHT_KINDS


def test_names_chunk_size_match() -> None:
    theirs = run_probe(LITELLM, "control", "{}")
    assert run_probe(BETTERLITE, "control", "{}") == theirs
    outcomes = STRINGS.validate_python(theirs["control"])
    assert len(set(outcomes)) > MIN_CONTROL_KINDS


def test_names_payload_to_json() -> None:
    theirs = run_probe(LITELLM, "payload", "{}")
    assert run_probe(BETTERLITE, "payload", "{}") == theirs
    assert theirs["plain"] != theirs["no_none"]
