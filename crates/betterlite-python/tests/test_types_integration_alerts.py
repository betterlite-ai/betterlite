import os
import subprocess
import sys
from typing import Final

import pytest
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError
from typing_extensions import TypedDict

from betterlite.types.integrations import (
    anthropic_cache_control_hook,
    slack_alerting,
    websearch_interception,
)
from betterlite.types.llms.base import LiteLLMPydanticObjectBase

DESCRIBE_SCRIPT = """
import dataclasses
import json
import os
import types
import typing
from enum import Enum

os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"

import typing_extensions
from pydantic import BaseModel

import betterlite.types.integrations.anthropic_cache_control_hook as betterlite_anthropic
import betterlite.types.integrations.code_interpreter_interception as betterlite_code
import betterlite.types.integrations.compression_interception as betterlite_compression
import betterlite.types.integrations.rag.bedrock_knowledgebase as betterlite_bedrock
import betterlite.types.integrations.slack_alerting as betterlite_slack
import betterlite.types.integrations.websearch_interception as betterlite_websearch
import litellm.types.integrations.anthropic_cache_control_hook as litellm_anthropic
import litellm.types.integrations.code_interpreter_interception as litellm_code
import litellm.types.integrations.compression_interception as litellm_compression
import litellm.types.integrations.rag.bedrock_knowledgebase as litellm_bedrock
import litellm.types.integrations.slack_alerting as litellm_slack
import litellm.types.integrations.websearch_interception as litellm_websearch
import litellm.types.llms.openai as litellm_openai

CONSTANTS = {
    "slack_alerting": (
        "DEFAULT_DIGEST_INTERVAL",
        "SLACK_ALERTING_THRESHOLD_5_PERCENT",
        "SLACK_ALERTING_THRESHOLD_15_PERCENT",
        "MAX_OLDEST_HANGING_REQUESTS_TO_CHECK",
        "HANGING_ALERT_BUFFER_TIME_SECONDS",
        "LITELLM_LOGO_URL",
        "LITELLM_SUPPORT_CONTACT",
        "DEFAULT_ALERT_TYPES",
    ),
    "anthropic_cache_control_hook": (
        "GATEWAY_INJECTED_CACHE_METADATA_KEY",
        "GATEWAY_INJECTED_FOR_EVERY_DEPLOYMENT",
    ),
    "code_interpreter_interception": (),
    "compression_interception": (),
    "websearch_interception": (),
    "rag.bedrock_knowledgebase": (),
}

MODULES = {
    "slack_alerting": (betterlite_slack, litellm_slack, ()),
    "anthropic_cache_control_hook": (
        betterlite_anthropic,
        litellm_anthropic,
        ("CacheControlInjectionPoint",),
    ),
    "code_interpreter_interception": (betterlite_code, litellm_code, ()),
    "compression_interception": (betterlite_compression, litellm_compression, ()),
    "websearch_interception": (
        betterlite_websearch,
        litellm_websearch,
        ("WebSearchToolResultErrorCode", "SearchOutcome"),
    ),
    "rag.bedrock_knowledgebase": (betterlite_bedrock, litellm_bedrock, ()),
}

EXTRAS = {
    "anthropic_cache_control_hook": {
        "ChatCompletionCachedContent": litellm_openai.ChatCompletionCachedContent,
    },
}


def normalize(value):
    return json.dumps(value, sort_keys=True)


def shape(annotation):
    if typing_extensions.is_typeddict(annotation):
        return {"typeddict": annotation.__name__}
    origin = typing.get_origin(annotation)
    if origin is None:
        return getattr(annotation, "__name__", str(annotation))
    arguments = typing.get_args(annotation)
    if origin is typing.Literal:
        return {"literal": sorted(normalize(argument) for argument in arguments)}
    if origin is typing.Union or origin is types.UnionType:
        return {"union": sorted(normalize(shape(argument)) for argument in arguments)}
    name = getattr(origin, "__name__", str(origin))
    if name == "dict" and len(arguments) == 2:
        if getattr(arguments[1], "__name__", "") in ("Any", "JsonValue"):
            return "dict"
        return {"dict": shape(arguments[1])}
    return {name: [shape(argument) for argument in arguments]}


def encode(value):
    if isinstance(value, Enum):
        return {"member": value.name, "value": encode(value.value)}
    if isinstance(value, (list, tuple)):
        return [encode(item) for item in value]
    if value is None or isinstance(value, (str, int, float, bool)):
        return value
    return {"object": type(value).__name__}


def enum_report(value):
    return {
        "canonical": [member.name for member in value],
        "members": {
            name: {"canonical": member.name, "value": encode(member.value)}
            for name, member in value.__members__.items()
        },
    }


def typed_dict_report(value):
    hints = typing_extensions.get_type_hints(value)
    return {
        "total": value.__total__,
        "required": sorted(value.__required_keys__),
        "optional": sorted(value.__optional_keys__),
        "readonly": sorted(getattr(value, "__readonly_keys__", ())),
        "annotations": [[name, shape(hint)] for name, hint in hints.items()],
    }


def field_report(field):
    found = {
        "annotation": shape(field.annotation),
        "required": field.is_required(),
        "description": field.description,
        "metadata": sorted(repr(item) for item in field.metadata),
    }
    if field.default_factory is not None:
        found["factory"] = field.default_factory.__name__
    elif not field.is_required():
        found["default"] = encode(field.default)
    return found


def model_report(model):
    return {
        "fields": [[name, field_report(field)] for name, field in model.model_fields.items()],
    }


def dataclass_report(value):
    found = []
    for field in dataclasses.fields(value):
        entry = {"name": field.name, "required": True}
        if field.default is not dataclasses.MISSING:
            entry = {"name": field.name, "required": False, "default": encode(field.default)}
        elif field.default_factory is not dataclasses.MISSING:
            entry = {
                "name": field.name,
                "required": False,
                "factory": field.default_factory.__name__,
            }
        found.append(entry)
    params = value.__dataclass_params__
    return {"fields": found, "frozen": params.frozen, "slots": params.slots}


def alias_report(module, name):
    if name not in vars(module):
        return None
    return shape(vars(module)[name])


def constants_report(module, names):
    found = {}
    for name in names:
        if name not in vars(module):
            found[name] = {"present": False}
        else:
            found[name] = {"present": True, "value": encode(vars(module)[name])}
    return found


def defined_here(module, name):
    value = vars(module).get(name)
    return value is not None and value.__module__ == module.__name__


def module_report(module, constants, aliases, extras):
    found = {
        "models": {},
        "typed_dicts": {},
        "dataclasses": {},
        "enums": {},
        "aliases": {},
        "constants": constants_report(module, constants),
        "extra": {},
        "extra_defined": sorted(name for name in extras if defined_here(module, name)),
    }
    for name, value in vars(module).items():
        if name.startswith("_") or name in extras:
            continue
        if not isinstance(value, type) or value.__module__ != module.__name__:
            continue
        if issubclass(value, BaseModel):
            found["models"][name] = model_report(value)
        elif issubclass(value, Enum):
            found["enums"][name] = enum_report(value)
        elif typing_extensions.is_typeddict(value):
            found["typed_dicts"][name] = typed_dict_report(value)
        elif dataclasses.is_dataclass(value):
            found["dataclasses"][name] = dataclass_report(value)
    for name, reference in extras.items():
        found["extra"][name] = typed_dict_report(vars(module).get(name, reference))
    for name in aliases:
        found["aliases"][name] = alias_report(module, name)
    return found


report = {"litellm": {}, "betterlite": {}}
for name, (ours, theirs, aliases) in MODULES.items():
    extras = EXTRAS.get(name, {})
    report["litellm"][name] = module_report(theirs, CONSTANTS[name], aliases, extras)
    report["betterlite"][name] = module_report(ours, CONSTANTS[name], aliases, extras)
print(json.dumps(report, sort_keys=True))
"""

FREQUENCY_SCRIPT: Final = (
    "import betterlite.types.integrations.slack_alerting as slack;"
    " fields = slack.SlackAlertingArgs.model_fields;"
    " print(repr(fields['daily_report_frequency'].default),"
    " slack.SlackAlertingArgs().daily_report_frequency)"
)


class ModuleReport(TypedDict):
    models: dict[str, JsonValue]
    typed_dicts: dict[str, JsonValue]
    dataclasses: dict[str, JsonValue]
    enums: dict[str, JsonValue]
    aliases: dict[str, JsonValue]
    constants: dict[str, JsonValue]
    extra: dict[str, JsonValue]
    extra_defined: list[str]


class Report(TypedDict):
    litellm: dict[str, ModuleReport]
    betterlite: dict[str, ModuleReport]


class ExpectedNames(TypedDict):
    models: frozenset[str]
    typed_dicts: frozenset[str]
    dataclasses: frozenset[str]
    enums: frozenset[str]
    aliases: frozenset[str]
    constants: frozenset[str]


MODULES: Final[tuple[str, ...]] = (
    "slack_alerting",
    "anthropic_cache_control_hook",
    "code_interpreter_interception",
    "compression_interception",
    "websearch_interception",
    "rag.bedrock_knowledgebase",
)

NO_NAMES: Final[frozenset[str]] = frozenset()

EXPECTED_NAMES: Final[dict[str, ExpectedNames]] = {
    "slack_alerting": {
        "models": frozenset(
            {"SlackAlertingArgs", "DeploymentMetrics", "HangingRequestData", "AlertTypeConfig"},
        ),
        "typed_dicts": frozenset(
            {
                "BaseOutageModel",
                "OutageModel",
                "ProviderRegionOutageModel",
                "AlertText",
                "AlertQueueItem",
                "DigestEntry",
            },
        ),
        "dataclasses": NO_NAMES,
        "enums": frozenset({"SlackAlertingArgsEnum", "SlackAlertingCacheKeys", "AlertType"}),
        "aliases": NO_NAMES,
        "constants": frozenset(
            {
                "DEFAULT_DIGEST_INTERVAL",
                "SLACK_ALERTING_THRESHOLD_5_PERCENT",
                "SLACK_ALERTING_THRESHOLD_15_PERCENT",
                "MAX_OLDEST_HANGING_REQUESTS_TO_CHECK",
                "HANGING_ALERT_BUFFER_TIME_SECONDS",
                "LITELLM_LOGO_URL",
                "LITELLM_SUPPORT_CONTACT",
                "DEFAULT_ALERT_TYPES",
            },
        ),
    },
    "anthropic_cache_control_hook": {
        "models": NO_NAMES,
        "typed_dicts": frozenset(
            {"CacheControlMessageInjectionPoint", "CacheControlToolConfigInjectionPoint"},
        ),
        "dataclasses": NO_NAMES,
        "enums": NO_NAMES,
        "aliases": frozenset({"CacheControlInjectionPoint"}),
        "constants": frozenset(
            {
                "GATEWAY_INJECTED_CACHE_METADATA_KEY",
                "GATEWAY_INJECTED_FOR_EVERY_DEPLOYMENT",
            },
        ),
    },
    "code_interpreter_interception": {
        "models": NO_NAMES,
        "typed_dicts": frozenset({"CodeInterpreterInterceptionConfig"}),
        "dataclasses": NO_NAMES,
        "enums": NO_NAMES,
        "aliases": NO_NAMES,
        "constants": NO_NAMES,
    },
    "compression_interception": {
        "models": NO_NAMES,
        "typed_dicts": frozenset(
            {"CompressionInterceptionConfig", "CompressionSavingsMetadata"},
        ),
        "dataclasses": NO_NAMES,
        "enums": NO_NAMES,
        "aliases": NO_NAMES,
        "constants": NO_NAMES,
    },
    "websearch_interception": {
        "models": frozenset({"AnthropicSearchQuery", "AnthropicServerToolUseBlock"}),
        "typed_dicts": frozenset({"RichWebSearchInput", "WebSearchInterceptionConfig"}),
        "dataclasses": frozenset({"SearchSucceeded", "SearchFailed"}),
        "enums": NO_NAMES,
        "aliases": frozenset({"WebSearchToolResultErrorCode", "SearchOutcome"}),
        "constants": NO_NAMES,
    },
    "rag.bedrock_knowledgebase": {
        "models": NO_NAMES,
        "typed_dicts": frozenset(
            {
                "BedrockKBLocation",
                "BedrockKBRowValue",
                "BedrockKBContent",
                "BedrockKBRetrievalResult",
                "BedrockKBResponse",
                "BedrockKBMetadataAttribute",
                "BedrockKBImplicitFilterConfiguration",
                "BedrockKBSelectiveModeConfiguration",
                "BedrockKBMetadataConfiguration",
                "BedrockKBModelConfiguration",
                "BedrockKBRerankingConfiguration",
                "BedrockKBVectorSearchConfiguration",
                "BedrockKBRetrievalConfiguration",
                "BedrockKBRetrievalQuery",
                "BedrockKBGuardrailConfiguration",
                "BedrockKBUserContext",
                "BedrockKBRequest",
            },
        ),
        "dataclasses": NO_NAMES,
        "enums": NO_NAMES,
        "aliases": NO_NAMES,
        "constants": NO_NAMES,
    },
}

DAILY_REPORT_FREQUENCY_DEFAULT: Final = 12 * 60 * 60
REPORT_CHECK_INTERVAL_DEFAULT: Final = 5 * 60
OUTAGE_ALERT_TTL_DEFAULT: Final = 1 * 60
CHECK_INTERVAL_MINIMUM: Final = 60
CHECK_INTERVAL_BELOW_MINIMUM: Final = 59


@pytest.fixture(scope="module")
def described() -> Report:
    completed = subprocess.run(
        [sys.executable, "-c", DESCRIBE_SCRIPT], check=True, capture_output=True, text=True,
    )
    report: Report = TypeAdapter(Report).validate_json(completed.stdout)
    return report


@pytest.mark.parametrize("module", MODULES)
def test_names_expected(described: Report, module: str) -> None:
    theirs = described["litellm"][module]
    expected = EXPECTED_NAMES[module]
    assert set(theirs["models"]) == expected["models"]
    assert set(theirs["typed_dicts"]) == expected["typed_dicts"]
    assert set(theirs["dataclasses"]) == expected["dataclasses"]
    assert set(theirs["enums"]) == expected["enums"]
    assert set(theirs["aliases"]) == expected["aliases"]
    assert set(theirs["constants"]) == expected["constants"]


@pytest.mark.parametrize("module", MODULES)
def test_names_sections_match(described: Report, module: str) -> None:
    ours = described["betterlite"][module]
    theirs = described["litellm"][module]
    assert ours["models"] == theirs["models"], module
    assert ours["typed_dicts"] == theirs["typed_dicts"], module
    assert ours["dataclasses"] == theirs["dataclasses"], module
    assert ours["enums"] == theirs["enums"], module
    assert ours["aliases"] == theirs["aliases"], module
    assert ours["constants"] == theirs["constants"], module
    assert ours["extra"] == theirs["extra"], module


# ast-grep-ignore: no-long-names
def test_names_local_chat_completion_cached_content(described: Report) -> None:
    ours = described["betterlite"]["anthropic_cache_control_hook"]
    theirs = described["litellm"]["anthropic_cache_control_hook"]
    assert ours["extra_defined"] == ["ChatCompletionCachedContent"]
    assert theirs["extra_defined"] == []
    assert set(ours["extra"]) == {"ChatCompletionCachedContent"}


# ast-grep-ignore: no-long-names
def test_names_slack_alerting_defaults() -> None:
    settings = slack_alerting.SlackAlertingArgs()
    assert settings.daily_report_frequency == DAILY_REPORT_FREQUENCY_DEFAULT
    assert settings.report_check_interval == REPORT_CHECK_INTERVAL_DEFAULT
    assert settings.outage_alert_ttl == OUTAGE_ALERT_TTL_DEFAULT
    assert settings.region_outage_alert_ttl == settings.outage_alert_ttl
    assert settings.log_to_console is False


# ast-grep-ignore: no-long-names
def test_names_slack_alerting_constraints() -> None:
    with pytest.raises(ValidationError):
        slack_alerting.SlackAlertingArgs(daily_spend_per_user_threshold=0)
    with pytest.raises(ValidationError):
        slack_alerting.SlackAlertingArgs(spend_anomaly_baseline_days=0)
    with pytest.raises(ValidationError):
        slack_alerting.SlackAlertingArgs(user_spend_check_interval=CHECK_INTERVAL_BELOW_MINIMUM)
    accepted = slack_alerting.SlackAlertingArgs(user_spend_check_interval=CHECK_INTERVAL_MINIMUM)
    assert accepted.user_spend_check_interval == CHECK_INTERVAL_MINIMUM


# ast-grep-ignore: no-long-names
def test_names_daily_report_frequency_env() -> None:
    configured = {**os.environ, "SLACK_DAILY_REPORT_FREQUENCY": "120"}
    overridden = subprocess.run(
        [sys.executable, "-c", FREQUENCY_SCRIPT],
        check=True,
        capture_output=True,
        text=True,
        env=configured,
    )
    assert overridden.stdout.strip() == "120 120"

    emptied = {**os.environ, "SLACK_DAILY_REPORT_FREQUENCY": ""}
    defaulted = subprocess.run(
        [sys.executable, "-c", FREQUENCY_SCRIPT],
        check=True,
        capture_output=True,
        text=True,
        env=emptied,
    )
    assert defaulted.stdout.strip() == "43200 43200"

    broken = {**os.environ, "SLACK_DAILY_REPORT_FREQUENCY": "not-a-number"}
    failed = subprocess.run(
        [sys.executable, "-c", FREQUENCY_SCRIPT],
        check=False,
        capture_output=True,
        text=True,
        env=broken,
    )
    assert failed.returncode != 0
    assert "SLACK_DAILY_REPORT_FREQUENCY" in failed.stderr


# ast-grep-ignore: no-long-names
def test_names_websearch_server_tool_use() -> None:
    block = websearch_interception.AnthropicServerToolUseBlock(
        id="srvtoolu_1",
        input=websearch_interception.AnthropicSearchQuery(query="weather"),
    )
    assert block.type == "server_tool_use"
    assert block.name == "web_search"
    assert block.input.query == "weather"


# ast-grep-ignore: no-long-names
def test_names_websearch_search_outcomes() -> None:
    succeeded = websearch_interception.SearchSucceeded(text="answer", response=None)
    failed = websearch_interception.SearchFailed(error_code="unavailable", message="down")
    assert succeeded.text == "answer"
    assert failed.error_code == "unavailable"


# ast-grep-ignore: no-long-names
def test_names_cache_control_injection_point() -> None:
    point: anthropic_cache_control_hook.CacheControlMessageInjectionPoint = {
        "location": "message",
        "role": "system",
        "index": 0,
        "control": {"type": "ephemeral", "ttl": "1h"},
    }
    assert point["control"] == {"type": "ephemeral", "ttl": "1h"}
    assert anthropic_cache_control_hook.GATEWAY_INJECTED_FOR_EVERY_DEPLOYMENT == ""


def test_names_pydantic_bases() -> None:
    assert slack_alerting.SlackAlertingArgs.__mro__[1] is LiteLLMPydanticObjectBase
    assert slack_alerting.DeploymentMetrics.__mro__[1] is LiteLLMPydanticObjectBase
    assert slack_alerting.AlertTypeConfig.__mro__[1] is LiteLLMPydanticObjectBase
    assert slack_alerting.HangingRequestData.__mro__[1] is BaseModel
