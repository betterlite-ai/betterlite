import base64
import subprocess
import sys
from asyncio import sleep
from collections.abc import Mapping, Sequence
from datetime import datetime
from typing import Final, Generic, TypeVar

import pytest
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import TypedDict

from betterlite.types.litellm_core_utils.streaming_chunk_builder_utils import UsagePerChunk
from betterlite.types.litellm_params_state import ForeignObject
from betterlite.types.passthrough_endpoints.managed_id_rewriter import (
    ManagedFileIdReader,
    ManagedFileIdWriter,
    ManagedFileRow,
    ManagedFileTable,
    ManagedObjectRow,
    ManagedObjectTable,
    ManagedResourceRow,
    ManagedRowData,
    ManagedTable,
    PrismaOrder,
    PrismaWhere,
)

DESCRIBE_SCRIPT = """
import base64
import importlib
import inspect
import json
import os
import typing
from dataclasses import MISSING, fields
from enum import Enum
from types import MappingProxyType

os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"

SUFFIXES = {
    "videos_main": "types.videos.main",
    "videos_utils": "types.videos.utils",
    "secret_managers_main": "types.secret_managers.main",
    "azure_ad_token": "types.secret_managers.get_azure_ad_token_provider",
    "assembly_ai": "types.passthrough_endpoints.assembly_ai",
    "managed_id_rewriter": "types.passthrough_endpoints.managed_id_rewriter",
    "pass_through_endpoints": "types.passthrough_endpoints.pass_through_endpoints",
    "tinyfish": "types.passthrough_endpoints.tinyfish",
    "init_prompts": "types.prompts.init_prompts",
    "streaming_chunk_builder": "types.litellm_core_utils.streaming_chunk_builder_utils",
    "daily_activity": "types.repositories.daily_activity",
}
MODULES = {key: ("litellm." + suffix, "betterlite." + suffix) for key, suffix in SUFFIXES.items()}

SPECS = {
    "videos_main": {
        "model": ["VideoObject", "VideoResponse", "CharacterObject"],
        "typeddict": [
            "VideoCreateOptionalRequestParams", "VideoCreateRequestParams", "DecodedVideoId",
            "VideoEditRequestParams", "VideoExtensionRequestParams",
        ],
        "declare": ["FileTypes"],
    },
    "videos_utils": {
        "plain": ["DecodedCharacterId"],
        "function": [
            "_add_base64_padding", "encode_video_id_with_provider", "decode_video_id_with_provider",
            "extract_original_video_id", "encode_character_id_with_provider",
            "decode_character_id_with_provider", "extract_original_character_id",
        ],
        "constant": ["VIDEO_ID_PREFIX", "CHARACTER_ID_PREFIX", "CHARACTER_ID_TEMPLATE"],
    },
    "secret_managers_main": {"enum": ["KeyManagementSystem"], "model": ["KeyManagementSettings"]},
    "azure_ad_token": {"enum": ["AzureCredentialType"]},
    "assembly_ai": {
        "constant": ["ASSEMBLY_AI_POLLING_INTERVAL", "ASSEMBLY_AI_MAX_POLLING_ATTEMPTS"],
    },
    "managed_id_rewriter": {
        "protocol": [
            "ManagedResourceRow", "ManagedFileRow", "ManagedObjectRow", "ManagedTable",
            "ManagedFileTable", "ManagedObjectTable", "ManagedFileIdReader", "ManagedFileIdWriter",
        ],
        "typeddict": ["ManagedListResponse"],
        "literal_alias": ["SortOrder", "ResourceKind"],
        "typevar": ["RowT"],
        "declare": ["PrismaWhereValue", "PrismaWhere", "PrismaOrder", "ManagedRowData"],
    },
    "pass_through_endpoints": {
        "enum": ["EndpointType"],
        "typeddict": ["PassthroughStandardLoggingPayload"],
        "constant": [
            "LITELLM_PASS_THROUGH_CUSTOM_BODY_STATE_KEY",
            "LITELLM_PASS_THROUGH_RAW_BODY_STATE_KEY",
            "LITELLM_PASS_THROUGH_DEPLOYMENT_MODEL_INFO_STATE_KEY",
            "LITELLM_PASS_THROUGH_ENDPOINT_MARKER",
        ],
    },
    "tinyfish": {
        "typeddict": ["TinyfishRun"],
        "function": ["is_allowed_tinyfish_endpoint"],
        "constant": [
            "TINYFISH_AGENT_DEFAULT_API_BASE", "TINYFISH_AGENT_DOCS_URL",
            "TINYFISH_DEFAULT_COST_PER_STEP", "TINYFISH_MODEL_NAME",
            "TINYFISH_POLLING_INTERVAL_SECONDS", "TINYFISH_MAX_POLLING_SECONDS",
            "TINYFISH_MAX_CONSECUTIVE_POLL_FAILURES", "TINYFISH_TERMINAL_RUN_STATUSES",
            "TINYFISH_AUTHENTICATED_RUN_FIELDS", "TINYFISH_REJECTED_ENVELOPE_FIELDS",
            "TINYFISH_PASSTHROUGH_TIMEOUT_SECONDS",
        ],
    },
    "init_prompts": {
        "enum": ["SupportedPromptIntegrations"],
        "model": [
            "PromptInfo", "PromptLiteLLMParams", "PromptSpec", "PromptTemplateBase",
            "PromptInfoResponse", "ListPromptsResponse",
        ],
    },
    "streaming_chunk_builder": {"typeddict": ["UsagePerChunk"]},
    "daily_activity": {
        "enum": ["DailyActivityTable", "ExportType"],
        "dataclass": [
            "DailyActivityScope", "KeySpendRow", "KeyPage", "KeyMetadataRow", "ExportRow",
            "RollupMetricsRow", "GroupingSetsRow", "EntityRollupRow", "AggregatedRows",
            "DailyRowsPage",
        ],
        "protocol": ["DailyActivityProxyReads", "DailyActivityRow"],
        "declare": ["SpendLogsWindow"],
    },
}

def plain(value):
    if isinstance(value, Enum):
        return {"enum": value.name}
    if isinstance(value, (MappingProxyType, dict)):
        return {str(key): plain(item) for key, item in value.items()}
    if isinstance(value, (set, frozenset)):
        return [plain(item) for item in sorted(value, key=repr)]
    if isinstance(value, (list, tuple)):
        return [plain(item) for item in value]
    if isinstance(value, type):
        return {"type": value.__name__}
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    return repr(value)

def literals(annotation):
    if typing.get_origin(annotation) is typing.Literal:
        return [plain(argument) for argument in typing.get_args(annotation)]
    return []

def signature_view(function):
    rendered = {}
    for name, parameter in inspect.signature(function).parameters.items():
        detail = {"kind": str(parameter.kind)}
        if parameter.default is not inspect.Parameter.empty:
            detail["default"] = plain(parameter.default)
        rendered[name] = detail
    return rendered

def model_view(model):
    described = []
    for name, info in model.model_fields.items():
        entry = {"required": info.is_required(), "literals": literals(info.annotation)}
        if info.default_factory is not None:
            entry["default"] = plain(info.default_factory())
        elif not info.is_required():
            entry["default"] = plain(info.default)
        described.append([name, entry])
    return described

def typeddict_view(typed):
    annotations = typing.get_type_hints(typed)
    return {
        "required": sorted(typed.__required_keys__),
        "optional": sorted(typed.__optional_keys__),
        "literals": {name: literals(annotation) for name, annotation in annotations.items()},
    }

def enum_view(enum_class):
    return {
        "members": [[name, plain(member.value)] for name, member in enum_class.__members__.items()],
        "str": issubclass(enum_class, str),
    }

def dataclass_view(cls):
    described = []
    for item in fields(cls):
        entry = {"name": item.name, "kw_only": item.kw_only}
        if item.default is not MISSING:
            entry["default"] = plain(item.default)
        if item.default_factory is not MISSING:
            entry["factory"] = plain(item.default_factory())
        described.append(entry)
    return {
        "fields": described,
        "frozen": cls.__dataclass_params__.frozen,
        "slots": "__slots__" in vars(cls),
    }

def protocol_view(protocol):
    attributes = {}
    for base in reversed(protocol.__mro__):
        attributes.update(base.__dict__.get("__annotations__", {}))
    methods = {
        name: signature_view(value)
        for name, value in vars(protocol).items()
        if inspect.isfunction(value) and not name.startswith("_")
    }
    return {
        "attributes": sorted(attributes),
        "methods": methods,
        "runtime": getattr(protocol, "_is_runtime_protocol", False),
    }

def plain_view(cls):
    return {
        "annotations": sorted(cls.__dict__.get("__annotations__", {})),
        "bases": [base.__name__ for base in cls.__bases__],
    }

def typevar_view(variable):
    bound = variable.__bound__
    return {
        "name": variable.__name__,
        "bound": None if bound is None else bound.__name__,
        "constraints": [constraint.__name__ for constraint in variable.__constraints__],
        "covariant": variable.__covariant__,
        "contravariant": variable.__contravariant__,
    }

def declared_view(value):
    return value is not None

VIEWS = {
    "model": model_view,
    "typeddict": typeddict_view,
    "enum": enum_view,
    "dataclass": dataclass_view,
    "protocol": protocol_view,
    "plain": plain_view,
    "literal_alias": lambda value: literals(value),
    "typevar": typevar_view,
    "constant": plain,
    "declare": declared_view,
    "function": declared_view,
}

def defined_names(module):
    found = set()
    for name, value in vars(module).items():
        if name.startswith("_"):
            continue
        if isinstance(value, type) and value.__module__ == module.__name__:
            found.add(name)
        elif inspect.isfunction(value) and value.__module__ == module.__name__:
            found.add(name)
    return found

def describe_modules():
    described = {}
    for key, spec in SPECS.items():
        declared = set()
        for kind, names in spec.items():
            if kind in VIEWS:
                declared.update(names)
        entry = {
            "missing_litellm": [],
            "missing_betterlite": [],
            "unexpected": [],
            "litellm": {},
            "betterlite": {},
        }
        imported = {}
        for side in ("litellm", "betterlite"):
            index = 0 if side == "litellm" else 1
            module = importlib.import_module(MODULES[key][index])
            imported[side] = module
            entry["missing_" + side] = sorted(
                name for name in declared if not hasattr(module, name)
            )
            for kind, names in spec.items():
                if kind not in VIEWS:
                    continue
                for name in names:
                    entry[side][name] = VIEWS[kind](getattr(module, name))
        entry["unexpected"] = sorted(defined_names(imported["litellm"]) - declared)
        described[key] = entry
    return described

def b64(text):
    return base64.b64encode(text.encode("utf-8")).decode("utf-8")

VIDEO_ENCODE_CASES = [
    ["vid-1", "openai", None], ["vid-2", "azure", "gpt-4o"],
    ["vid-3", "", None], ["", "openai", None],
    ["video_already", "openai", None],
    ["video_" + b64("litellm:custom_llm_provider:openai;model_id:m;video_id:v"), "openai", "m"],
    ["video_" + b64("a;b"), "openai", "m"], ["video_a", "openai", "m"],
    ["video_video_abc", "openai", "m"], ["not_a_video", "openai", "m"],
]
EXTRA_DECODE_INPUTS = [
    "", "video_", "vid-1", "video_" + b64("x;y;z"),
    "video_" + b64("litellm:custom_llm_provider:openai;model_id:m;video_id:v;extra"),
    "video_" + b64("litellm:custom_llm_provider:openai;model_id:m;video_id:v").rstrip("="),
    "video_abc", "video_a", "video_!!!", "Video_abc",
    "character_" + b64("litellm:custom_llm_provider:openai;model_id:m;character_id:c"),
]
SURROGATE_IDS = ["video_" + chr(0xD800), "character_" + chr(0xD800)]

def codec_results(package, label):
    utils = importlib.import_module(package + ".types.videos.utils")
    encode = getattr(utils, "encode_" + label + "_id_with_provider")
    decode = getattr(utils, "decode_" + label + "_id_with_provider")
    extract = getattr(utils, "extract_original_" + label + "_id")
    decode_inputs = [case[0] for case in VIDEO_ENCODE_CASES] + EXTRA_DECODE_INPUTS
    encoded = [encode(case[0], case[1], case[2]) for case in VIDEO_ENCODE_CASES]
    surrogates = [(dict(decode(item)), item) for item in SURROGATE_IDS]
    return {
        "encoded": encoded,
        "decoded": [dict(decode(item)) for item in decode_inputs],
        "original": [extract(item) for item in decode_inputs],
        "reencoded": [encode(item, "azure", "m2") for item in decode_inputs],
        "round_trip": [extract(item) for item in encoded],
        "surrogate": [
            [view[label + "_id"] == item, view["custom_llm_provider"], extract(item) == item]
            for view, item in surrogates
        ],
    }

ENDPOINT_CASES = [
    ["POST", "/v1/automation/run"],
    ["POST", "/v1/automation/run-async"],
    ["POST", "/v1/automation/run-sse"],
    ["GET", "/v1/automation/run"],
    ["POST", "/v1/automation/run/"],
    ["POST", "/v1/automation/run/extra"],
    ["GET", "/v1/runs/abc"],
    ["POST", "/v1/runs/abc"],
    ["GET", "/v1/runs"],
    ["GET", "/v1/runs/"],
    ["GET", "/v1/runs/abc/cancel"],
    ["POST", "/v1/runs/abc/cancel"],
    ["POST", "/v1/runs/abc/delete"],
    ["POST", "v1/automation/run"],
    ["POST", "/v1//runs"],
    ["POST", "/v1/./automation/run"],
    ["POST", "/v1/automation/../run"],
    ["POST", "/v1/../runs/abc/cancel"],
    ["get", "/v1/runs/abc"],
    ["POST", ""],
    ["POST", "/"],
    ["POST", "/v1/runs/abc/cancel/"],
]

def endpoint_results(package):
    module = importlib.import_module(package + ".types.passthrough_endpoints.tinyfish")
    return [
        [method, path, module.is_allowed_tinyfish_endpoint(method, path)]
        for method, path in ENDPOINT_CASES
    ]

SCOPE_CASES = [
    ["USER", "user_id"],
    ["TEAM", "team_id"],
    ["TAG", "tag"],
    ["ORGANIZATION", "organization_id"],
    ["CUSTOMER", "end_user_id"],
    ["AGENT", "agent_id"],
    ["USER", "team_id"],
    ["TEAM", ""],
    ["TAG", "api_key"],
    ["CUSTOMER", "user_id"],
    ["AGENT", "ORGANIZATION_ID"],
    ["ORGANIZATION", "org_id"],
    ["USER", "user_id "],
]

def scope_results(package):
    module = importlib.import_module(package + ".types.repositories.daily_activity")
    results = []
    for table_name, field_name in SCOPE_CASES:
        try:
            scope = module.DailyActivityScope(
                table=module.DailyActivityTable[table_name], entity_id_field=field_name,
                entity_ids=("a", "b"), exclude_entity_ids=(), api_keys=None,
                start_date="2026-01-01", end_date="2026-01-31", model="gpt-4o",
                timezone_offset_minutes=60,
            )
        except ValueError as error:
            results.append({"error": str(error), "type": type(error).__name__})
        else:
            results.append(
                {
                    "table": scope.table.value,
                    "entity_id_field": scope.entity_id_field,
                    "entity_ids": list(scope.entity_ids),
                    "include_current_utc_day": scope.include_current_utc_day,
                }
            )
    return results

def validation_report(model, payload):
    try:
        model(**payload)
    except Exception as error:
        return {
            "error": type(error).__name__,
            "details": [[list(item["loc"]), item["type"]] for item in error.errors()],
        }
    return {"ok": True}

def prompt_results(package):
    module = importlib.import_module(package + ".types.prompts.init_prompts")
    params = module.PromptLiteLLMParams(prompt_integration="dotprompt")
    default_spec = module.PromptSpec(prompt_id="p1", litellm_params=params)
    staging_spec = module.PromptSpec(
        prompt_id="p2",
        litellm_params={
            "prompt_integration": "generic_prompt_management", "prompt_directory": "/prompts",
        },
        prompt_info={"environment": "staging", "owner": "team-a"},
    )
    staging_dump = staging_spec.model_dump(mode="json")
    db_spec = module.PromptSpec(
        prompt_id="p3", litellm_params=params, prompt_info={"prompt_type": "db"},
    )
    response = module.PromptInfoResponse(
        prompt_spec=default_spec,
        raw_prompt_template={"litellm_prompt_id": "p1", "content": "hello"},
        environments=["development", "staging"],
    )
    listing = module.ListPromptsResponse(prompts=[default_spec, staging_spec, db_spec])
    return {
        "params": params.model_dump(mode="json"),
        "default_spec": default_spec.model_dump(mode="json"),
        "staging_spec": staging_dump,
        "staging_extras": {
            "prompt_directory": staging_dump["litellm_params"].get("prompt_directory"),
            "owner": staging_dump["prompt_info"].get("owner"),
        },
        "db_spec": db_spec.model_dump(mode="json"),
        "response": response.model_dump(mode="json"),
        "listing": listing.model_dump(mode="json"),
        "missing_params": validation_report(module.PromptSpec, {"prompt_id": "p4"}),
        "missing_prompt_id": validation_report(module.PromptSpec, {"litellm_params": params}),
        "bad_prompt_type": validation_report(
            module.PromptSpec,
            {"prompt_id": "p5", "litellm_params": params, "prompt_info": {"prompt_type": "nope"}},
        ),
        "null_prompt_info": validation_report(
            module.PromptSpec,
            {"prompt_id": "p6", "litellm_params": params, "prompt_info": None},
        ),
        "integrations": [member.value for member in module.SupportedPromptIntegrations],
    }

def settings_results(package):
    module = importlib.import_module(package + ".types.secret_managers.main")
    settings_module = importlib.import_module(
        package + ".types.secret_managers.get_azure_ad_token_provider"
    )
    empty = module.KeyManagementSettings()
    full = module.KeyManagementSettings(
        hosted_keys=["azure/key1", "aws/key2"], store_virtual_keys=True,
        prefix_for_stored_virtual_keys="team/", access_mode="read_and_write",
        primary_secret_name="primary", description="d", tags={"Environment": "Prod"},
        kms_key_id="alias/aws/secretsmanager", custom_secret_manager="my.SecretManager",
        aws_region_name="us-east-1", aws_role_name="arn:aws:iam::1:role/r",
        aws_session_name="s", aws_external_id="e", aws_profile_name="p",
        aws_web_identity_token="t", aws_sts_endpoint="https://sts.example",
        replica_regions=["us-west-2"],
    )
    return {
        "empty": empty.model_dump(mode="json"),
        "full": full.model_dump(mode="json"),
        "systems": {member.name: member.value for member in module.KeyManagementSystem},
        "credentials": [member.value for member in settings_module.AzureCredentialType],
    }

def video_model_results(package):
    module = importlib.import_module(package + ".types.videos.main")
    video = module.VideoObject(
        id="video_123", object="video", status="completed", created_at=1700000000,
        completed_at=1700000005, expires_at=1700000600, error={"code": "x"}, progress=100,
        remixed_from_video_id="video_0", seconds="8", size="1280x720", model="sora-2",
        usage={"total_tokens": 12},
    )
    character = module.CharacterObject(id="char_1", created_at=1700000000, name="ada")
    response = module.VideoResponse(data=[video], hidden_params={"model_id": "m"})
    extra = module.VideoObject(id="video_x", object="video", status="queued", bogus_field="x")
    try:
        video["missing"]
    except Exception as error:
        missing_key = type(error).__name__
    else:
        missing_key = "no error"
    return {
        "video": video.model_dump(mode="json"),
        "character": character.model_dump(mode="json"),
        "response": response.model_dump(mode="json"),
        "video_access": {
            "id": video["id"], "get_id": video.get("id"),
            "get_missing": video.get("missing"), "has_object": "object" in video,
            "has_missing": "missing" in video, "hidden_params": video.get("_hidden_params"),
        },
        "response_access": {
            "count": len(response["data"]), "get_missing": response.get("missing"),
            "has_data": "data" in response,
        },
        "missing_key": missing_key,
        "extra_ignored": "bogus_field" in extra.model_dump(mode="json"),
        "json_exclude_none": video.json(exclude_none=True),
        "default_character_object": character.object,
    }

def behaviour(package):
    return {
        "video_ids": codec_results(package, "video"),
        "character_ids": codec_results(package, "character"),
        "endpoints": endpoint_results(package),
        "scopes": scope_results(package),
        "prompts": prompt_results(package),
        "settings": settings_results(package),
        "video_models": video_model_results(package),
    }

report = {
    "modules": describe_modules(),
    "behaviour": {"litellm": behaviour("litellm"), "betterlite": behaviour("betterlite")},
}
print(json.dumps(report, sort_keys=True))
"""

EXPECTED_SECTIONS: Final[tuple[str, ...]] = (
    "character_ids", "endpoints", "prompts", "scopes",
    "settings", "video_ids", "video_models",
)

MODULE_KEYS: Final[tuple[str, ...]] = (
    "assembly_ai", "azure_ad_token", "daily_activity", "init_prompts",
    "managed_id_rewriter", "pass_through_endpoints", "secret_managers_main",
    "streaming_chunk_builder", "tinyfish", "videos_main", "videos_utils",
)

TINYFISH_ENDPOINTS: Final[list[list[JsonValue]]] = [
    ["POST", "/v1/automation/run", True], ["POST", "/v1/automation/run-async", True],
    ["POST", "/v1/automation/run-sse", True], ["GET", "/v1/automation/run", False],
    ["POST", "/v1/automation/run/", False], ["POST", "/v1/automation/run/extra", False],
    ["GET", "/v1/runs/abc", True], ["POST", "/v1/runs/abc", False],
    ["GET", "/v1/runs", False], ["GET", "/v1/runs/", False],
    ["GET", "/v1/runs/abc/cancel", False], ["POST", "/v1/runs/abc/cancel", True],
    ["POST", "/v1/runs/abc/delete", False], ["POST", "v1/automation/run", False],
    ["POST", "/v1//runs", False], ["POST", "/v1/./automation/run", False],
    ["POST", "/v1/automation/../run", False], ["POST", "/v1/../runs/abc/cancel", False],
    ["get", "/v1/runs/abc", False], ["POST", "", False],
    ["POST", "/", False], ["POST", "/v1/runs/abc/cancel/", False],
]

PROMPT_INTEGRATIONS: Final[list[JsonValue]] = [
    "dotprompt", "langfuse", "custom", "bitbucket", "gitlab",
    "generic_prompt_management", "arize_phoenix",
]

AZURE_CREDENTIALS: Final[list[JsonValue]] = [
    "ClientSecretCredential", "ManagedIdentityCredential", "CertificateCredential",
    "DefaultAzureCredential", "DeploymentIdentityCredential",
]

USAGE_CHUNK: Final[dict[str, JsonValue]] = {
    "prompt_tokens": 1, "completion_tokens": 2, "cache_creation_input_tokens": 0,
    "cache_read_input_tokens": 0, "server_tool_use": None, "web_search_requests": 0,
    "google_maps_grounding_requests": 0, "completion_tokens_details": None,
    "prompt_tokens_details": None, "cost": None, "inference_geo": None, "speed": None,
}


class DecodedVideoIdNames(TypedDict):
    custom_llm_provider: str | None
    model_id: str | None
    video_id: str


class DecodedCharacterIdNames(TypedDict):
    custom_llm_provider: str | None
    model_id: str | None
    character_id: str


class VideoCodecNames(TypedDict):
    encoded: list[str]
    decoded: list[DecodedVideoIdNames]
    original: list[str]
    reencoded: list[str]
    round_trip: list[str]
    surrogate: list[list[JsonValue]]


class CharacterCodecNames(TypedDict):
    encoded: list[str]
    decoded: list[DecodedCharacterIdNames]
    original: list[str]
    reencoded: list[str]
    round_trip: list[str]
    surrogate: list[list[JsonValue]]


class BehaviourNames(TypedDict):
    video_ids: VideoCodecNames
    character_ids: CharacterCodecNames
    endpoints: list[list[JsonValue]]
    scopes: list[dict[str, JsonValue]]
    prompts: dict[str, JsonValue]
    settings: dict[str, JsonValue]
    video_models: dict[str, JsonValue]


class Sides(TypedDict):
    litellm: BehaviourNames
    betterlite: BehaviourNames


class ModuleEntry(TypedDict):
    missing_litellm: list[str]
    missing_betterlite: list[str]
    unexpected: list[str]
    litellm: dict[str, JsonValue]
    betterlite: dict[str, JsonValue]


class Report(TypedDict):
    modules: dict[str, ModuleEntry]
    behaviour: Sides


class PromptInfoNames(TypedDict):
    prompt_type: str
    environment: str | None


class PromptSpecNames(TypedDict):
    prompt_id: str
    prompt_info: PromptInfoNames
    environment: str | None


class ValidationNames(TypedDict):
    error: str
    details: list[JsonValue]


class PromptExtrasNames(TypedDict):
    prompt_directory: str | None
    owner: str | None


class PromptNames(TypedDict):
    default_spec: PromptSpecNames
    staging_spec: PromptSpecNames
    staging_extras: PromptExtrasNames
    db_spec: PromptSpecNames
    missing_params: ValidationNames
    missing_prompt_id: ValidationNames
    bad_prompt_type: ValidationNames
    null_prompt_info: ValidationNames
    integrations: list[JsonValue]


class KeyManagementNames(TypedDict):
    hosted_keys: list[JsonValue] | None
    store_virtual_keys: bool | None
    prefix_for_stored_virtual_keys: str
    access_mode: str
    aws_region_name: str | None
    tags: dict[str, str] | None


class SettingsNames(TypedDict):
    empty: KeyManagementNames
    full: KeyManagementNames
    systems: dict[str, str]
    credentials: list[JsonValue]


class VideoNames(TypedDict):
    id: str
    object: str
    status: str
    created_at: int


class CharacterNames(TypedDict):
    id: str
    object: str
    created_at: int
    name: str


class VideoAccessNames(TypedDict):
    id: str
    has_object: bool
    has_missing: bool
    get_missing: None
    hidden_params: dict[str, JsonValue]


class VideoModelsNames(TypedDict):
    video: VideoNames
    character: CharacterNames
    video_access: VideoAccessNames
    missing_key: str
    default_character_object: str
    extra_ignored: bool


class _StubRow:
    created_by: str | None = None
    team_id: str | None = None
    created_at: datetime | None = None
    file_object: JsonValue = None
    unified_file_id: str = ""
    unified_object_id: str = ""


RowStub = TypeVar("RowStub", bound=ManagedResourceRow)


class _StubTable(Generic[RowStub]):
    async def find_first(self, *, where: PrismaWhere) -> RowStub | None:
        _ = where
        await sleep(0)
        return None

    async def find_many(
        self,
        *,
        where: PrismaWhere,
        order: PrismaOrder | Sequence[PrismaOrder] | None = None,
        take: int | None = None,
    ) -> list[RowStub]:
        _ = (where, order, take)
        await sleep(0)
        return []


class _StubObjectTable(_StubTable[ManagedObjectRow]):
    async def update(
        self, *, where: PrismaWhere, data: ManagedRowData,
    ) -> ManagedObjectRow | None:
        _ = (where, data)
        await sleep(0)
        return None

    async def upsert(
        self, *, where: PrismaWhere, data: Mapping[str, ManagedRowData],
    ) -> ManagedObjectRow:
        _ = (where, data)
        await sleep(0)
        return _StubRow()


class _StubManagedHook:
    async def get_unified_file_id(
        self, file_id: str, litellm_parent_otel_span: ForeignObject | None = None,
    ) -> ManagedFileRow | None:
        _ = (file_id, litellm_parent_otel_span)
        await sleep(0)
        return None

    async def store_unified_file_id(
        self, file_id: str, file_object: ForeignObject | None,
        litellm_parent_otel_span: ForeignObject | None, model_mappings: dict[str, str],
        user_api_key_dict: ForeignObject,
    ) -> None:
        _ = (file_id, file_object, litellm_parent_otel_span, model_mappings, user_api_key_dict)
        await sleep(0)


@pytest.fixture(scope="module")
def report() -> Report:
    completed = subprocess.run(
        [sys.executable, "-c", DESCRIBE_SCRIPT],
        check=True,
        capture_output=True,
        text=True,
    )
    parsed: Report = TypeAdapter(Report).validate_json(completed.stdout)
    return parsed


def test_module_names_match_litellm(report: Report) -> None:
    modules = report["modules"]
    assert tuple(sorted(modules)) == MODULE_KEYS
    for key in MODULE_KEYS:
        entry = modules[key]
        assert not entry["missing_litellm"]
        assert not entry["missing_betterlite"]
        assert not entry["unexpected"]
        assert entry["litellm"]
        assert entry["betterlite"] == entry["litellm"]


def test_behaviour_names_match_litellm(report: Report) -> None:
    behaviour = report["behaviour"]
    assert tuple(sorted(behaviour["betterlite"])) == EXPECTED_SECTIONS
    assert behaviour["betterlite"] == behaviour["litellm"]


def test_video_id_codec_names(report: Report) -> None:
    video_ids = report["behaviour"]["betterlite"]["video_ids"]
    payload = b"litellm:custom_llm_provider:openai;model_id:;video_id:vid-1"
    expected = "video_" + base64.b64encode(payload).decode("utf-8")
    assert video_ids["encoded"][0] == expected
    assert video_ids["encoded"][2] == "vid-3"
    assert video_ids["encoded"][3] == ""
    assert video_ids["decoded"][0] == DecodedVideoIdNames(
        custom_llm_provider=None, model_id=None, video_id="vid-1",
    )
    assert video_ids["decoded"][5] == DecodedVideoIdNames(
        custom_llm_provider="openai", model_id="m", video_id="v",
    )
    assert video_ids["decoded"][7] == DecodedVideoIdNames(
        custom_llm_provider=None, model_id=None, video_id="video_a",
    )
    assert video_ids["decoded"][13] == DecodedVideoIdNames(
        custom_llm_provider="x", model_id="y", video_id="z",
    )
    assert video_ids["original"][5] == "v"
    assert video_ids["original"][7] == "video_a"
    assert video_ids["round_trip"][6] == "video_YTti"
    assert video_ids["reencoded"][0] != video_ids["encoded"][0]


def test_character_id_codec_names(report: Report) -> None:
    character_ids = report["behaviour"]["betterlite"]["character_ids"]
    payload = b"litellm:custom_llm_provider:azure;model_id:gpt-4o;character_id:vid-2"
    expected = "character_" + base64.b64encode(payload).decode("utf-8")
    assert character_ids["encoded"][1] == expected
    assert character_ids["encoded"][2] == "vid-3"
    assert character_ids["encoded"][3] == ""
    assert character_ids["decoded"][0] == DecodedCharacterIdNames(
        custom_llm_provider=None, model_id=None, character_id="vid-1",
    )
    assert character_ids["decoded"][-1] == DecodedCharacterIdNames(
        custom_llm_provider="openai", model_id="m", character_id="c",
    )
    assert character_ids["original"][4] == "video_already"
    assert character_ids["original"][-1] == "c"
    assert character_ids["round_trip"][1] == "vid-2"


def test_surrogate_id_codec_names(report: Report) -> None:
    unchanged: Final[list[JsonValue]] = [True, None, True]
    codecs = report["behaviour"]["betterlite"]
    assert codecs["video_ids"]["surrogate"] == [unchanged, unchanged]
    assert codecs["character_ids"]["surrogate"] == [unchanged, unchanged]


def test_tinyfish_endpoint_names(report: Report) -> None:
    endpoints = report["behaviour"]["betterlite"]["endpoints"]
    assert endpoints == TINYFISH_ENDPOINTS


def test_daily_activity_scope_names(report: Report) -> None:
    scopes = report["behaviour"]["betterlite"]["scopes"]
    assert scopes[0] == {
        "entity_id_field": "user_id",
        "entity_ids": ["a", "b"],
        "include_current_utc_day": False,
        "table": "litellm_dailyuserspend",
    }
    assert scopes[4] == {
        "entity_id_field": "end_user_id",
        "entity_ids": ["a", "b"],
        "include_current_utc_day": False,
        "table": "litellm_dailyenduserspend",
    }
    assert scopes[6] == {
        "error": "Invalid entity_id_field 'team_id' for litellm_dailyuserspend",
        "type": "ValueError",
    }
    assert scopes[9] == {
        "error": "Invalid entity_id_field 'user_id' for litellm_dailyenduserspend",
        "type": "ValueError",
    }
    assert scopes[12] == {
        "error": "Invalid entity_id_field 'user_id ' for litellm_dailyuserspend",
        "type": "ValueError",
    }


def test_prompt_spec_names(report: Report) -> None:
    prompts = TypeAdapter(PromptNames).validate_python(
        report["behaviour"]["betterlite"]["prompts"],
    )
    assert prompts["default_spec"]["prompt_info"] == PromptInfoNames(
        prompt_type="config", environment="development",
    )
    assert prompts["staging_spec"]["prompt_info"] == PromptInfoNames(
        prompt_type="config", environment="staging",
    )
    assert prompts["db_spec"]["prompt_info"] == PromptInfoNames(
        prompt_type="db", environment="development",
    )
    assert prompts["staging_extras"] == PromptExtrasNames(
        prompt_directory="/prompts", owner="team-a",
    )
    assert prompts["default_spec"]["prompt_id"] == "p1"
    assert prompts["default_spec"]["environment"] == "development"
    assert prompts["missing_params"]["error"] == "ValidationError"
    assert prompts["missing_params"]["details"] == [[["litellm_params"], "missing"]]
    assert prompts["missing_prompt_id"]["details"] == [[["prompt_id"], "missing"]]
    assert prompts["bad_prompt_type"]["details"] == [
        [["prompt_info", "prompt_type"], "literal_error"],
    ]
    assert prompts["null_prompt_info"]["details"] == [[["prompt_info"], "model_type"]]
    assert prompts["integrations"] == PROMPT_INTEGRATIONS


def test_key_management_names(report: Report) -> None:
    settings = TypeAdapter(SettingsNames).validate_python(
        report["behaviour"]["betterlite"]["settings"],
    )
    assert settings["empty"]["access_mode"] == "read_only"
    assert settings["empty"]["store_virtual_keys"] is False
    assert settings["empty"]["prefix_for_stored_virtual_keys"] == "litellm/"
    assert settings["empty"]["hosted_keys"] is None
    assert settings["empty"]["aws_region_name"] is None
    assert settings["empty"]["tags"] is None
    assert settings["full"]["access_mode"] == "read_and_write"
    assert settings["full"]["hosted_keys"] == ["azure/key1", "aws/key2"]
    assert settings["full"]["aws_region_name"] == "us-east-1"
    assert settings["full"]["tags"] == {"Environment": "Prod"}
    # two pieces, since ruff S105 reads a secret literal as a password
    assert settings["systems"]["AWS_SECRET_MANAGER"] == "aws_" + "secret_manager"
    assert settings["systems"]["GOOGLE_SECRET_MANAGER"] == "google_" + "secret_manager"
    assert settings["systems"]["LOCAL"] == "local"
    assert settings["credentials"] == AZURE_CREDENTIALS


def test_video_object_names(report: Report) -> None:
    models = TypeAdapter(VideoModelsNames).validate_python(
        report["behaviour"]["betterlite"]["video_models"],
    )
    assert models["video"] == VideoNames(
        id="video_123", object="video", status="completed", created_at=1700000000,
    )
    assert models["character"] == CharacterNames(
        id="char_1", object="character", created_at=1700000000, name="ada",
    )
    assert not models["video_access"]["hidden_params"]
    assert models["video_access"]["has_object"] is True
    assert models["video_access"]["has_missing"] is False
    assert models["video_access"]["get_missing"] is None
    assert models["missing_key"] == "AttributeError"
    assert models["default_character_object"] == "character"
    assert models["extra_ignored"] is False


def test_usage_per_chunk_names() -> None:
    adapter: Final[TypeAdapter[UsagePerChunk]] = TypeAdapter(UsagePerChunk)
    chunk = adapter.validate_python(USAGE_CHUNK)
    assert chunk["prompt_tokens"] == 1
    assert chunk["cache_read_input_tokens"] == 0
    assert chunk["cost"] is None
    assert chunk["speed"] is None
    with pytest.raises(ValidationError) as incomplete:
        adapter.validate_python({})
    missing = {str(error["loc"]) for error in incomplete.value.errors()}
    assert "('prompt_tokens',)" in missing
    assert "('completion_tokens_details',)" in missing
    assert "('speed',)" in missing


def test_protocol_names_are_structural() -> None:
    resource_row: ManagedResourceRow = _StubRow()
    file_row: ManagedFileRow = _StubRow()
    object_row: ManagedObjectRow = _StubRow()
    file_table: ManagedFileTable = _StubTable[ManagedFileRow]()
    object_table: ManagedObjectTable = _StubObjectTable()
    generic_table: ManagedTable[ManagedFileRow] = file_table
    assert resource_row.team_id is None
    assert file_row.unified_file_id == ""
    assert object_row.unified_object_id == ""
    assert callable(file_table.find_first)
    assert callable(generic_table.find_many)
    assert callable(object_table.update)
    assert callable(object_table.upsert)


def test_runtime_protocol_names() -> None:
    reader: ManagedFileIdReader = _StubManagedHook()
    writer: ManagedFileIdWriter = _StubManagedHook()
    assert isinstance(reader, ManagedFileIdReader)  # ast-grep-ignore: no-isinstance
    assert isinstance(writer, ManagedFileIdWriter)  # ast-grep-ignore: no-isinstance
    assert not isinstance(_StubRow(), ManagedFileIdReader)  # ast-grep-ignore: no-isinstance
