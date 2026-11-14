import json
import subprocess
import sys
from typing import NotRequired

import pytest
from pydantic import TypeAdapter
from typing_extensions import TypedDict

from betterlite.types.utils import JsonValue as Json

ROI_MODULE = "types.roi_calculator"
MEMORY_MODULE = "types.memory_management"
INSIGHT_MODULE = "types.model_insights"
MODULES = (ROI_MODULE, MEMORY_MODULE, INSIGHT_MODULE)
DESCRIBE_TIMEOUT_SECONDS = 300

DESCRIBE_SCRIPT = """
import importlib
import json
import sys
import types
import typing
from collections.abc import Mapping
from datetime import datetime

import typing_extensions
from pydantic import BaseModel, SecretStr, ValidationError
from typing_extensions import NotRequired, ReadOnly, Required

MODULES = ("types.roi_calculator", "types.memory_management", "types.model_insights")
WRAPPERS = (NotRequired, ReadOnly, Required)


def unwrap(annotation):
    while typing.get_origin(annotation) in WRAPPERS:
        annotation = typing.get_args(annotation)[0]
    return annotation


def literals(annotation):
    origin = typing.get_origin(annotation)
    if origin is typing.Literal:
        return list(typing.get_args(annotation))
    if origin in (typing.Union, types.UnionType):
        values = []
        for argument in typing.get_args(annotation):
            if argument is type(None):
                continue
            nested = literals(argument)
            if nested is None:
                return None
            values.extend(nested)
        return values
    return None


def encode(value):
    if value is None:
        return {"kind": "none"}
    if isinstance(value, SecretStr):
        return {"kind": "secret", "value": value.get_secret_value()}
    if isinstance(value, datetime):
        return {"kind": "datetime", "value": value.isoformat()}
    if isinstance(value, Mapping):
        encoded = {key: encode(item) for key, item in value.items()}
        return {"kind": "mapping", "value": encoded}
    if isinstance(value, tuple):
        return {"kind": "tuple", "value": [encode(item) for item in value]}
    if isinstance(value, list):
        return {"kind": "list", "value": [encode(item) for item in value]}
    return {"kind": "plain", "value": value}


def local_bases(cls, module_name):
    return [
        base.__name__
        for base in cls.__bases__
        if getattr(base, "__module__", None) == module_name
    ]


def describe_model(model):
    fields = []
    for name, info in model.model_fields.items():
        default = {"kind": "required"} if info.is_required() else encode(
            info.get_default(call_default_factory=True)
        )
        fields.append(
            {
                "name": name,
                "required": info.is_required(),
                "default": default,
                "description": info.description,
                "literals": literals(info.annotation),
            }
        )
    config = model.model_config
    return {
        "kind": "model",
        "bases": local_bases(model, model.__module__),
        "config": {
            "extra": config.get("extra"),
            "frozen": config.get("frozen"),
            "strict": config.get("strict"),
            "from_attributes": config.get("from_attributes"),
        },
        "fields": fields,
    }


def describe_typeddict(typed_dict):
    fields = []
    for name, annotation in typed_dict.__annotations__.items():
        fields.append(
            {
                "name": name,
                "required": name in typed_dict.__required_keys__,
                "default": {"kind": "absent"},
                "description": None,
                "literals": literals(unwrap(annotation)),
            }
        )
    return {
        "kind": "typeddict",
        "bases": local_bases(typed_dict, typed_dict.__module__),
        "config": {"total": typed_dict.__total__},
        "fields": fields,
    }


def describe_module(package, module_name):
    module = importlib.import_module(package + "." + module_name)
    # a package such as betterlite's roi_calculator defines its names in submodules
    owner = module.__name__ + "."
    classes = {}
    aliases = {}
    for name in dir(module):
        if name.startswith("_"):
            continue
        value = getattr(module, name)
        if isinstance(value, type) and issubclass(value, BaseModel):
            if (value.__module__ + ".").startswith(owner):
                classes[name] = describe_model(value)
            continue
        if typing_extensions.is_typeddict(value):
            if (value.__module__ + ".").startswith(owner):
                classes[name] = describe_typeddict(value)
            continue
        if typing.get_origin(value) is typing.Literal:
            aliases[name] = {"literals": list(typing.get_args(value))}
    return {"classes": classes, "aliases": aliases}


def run_probe(package, probe):
    module = importlib.import_module(package + "." + probe["module"])
    target = getattr(module, probe["target"])
    try:
        if isinstance(target, type) and issubclass(target, BaseModel):
            dumped = target.model_validate(probe["payload"]).model_dump()
            return {"outcome": "value", "value": encode(dumped)}
        return {"outcome": "value", "value": encode(target(probe["payload"]))}
    except ValidationError as error:
        return {
            "outcome": "error",
            "value": {
                "kind": "ValidationError",
                "errors": [
                    {"type": item["type"], "loc": list(item["loc"]), "msg": item["msg"]}
                    for item in error.errors(include_url=False)
                ],
            },
        }
    except ValueError as error:
        return {
            "outcome": "error",
            "value": {"kind": type(error).__name__, "message": str(error)},
        }


def main():
    package = sys.argv[1]
    probes = json.loads(sys.stdin.read())
    result = {
        "modules": {name: describe_module(package, name) for name in MODULES},
        "probes": [run_probe(package, probe) for probe in probes],
    }
    print(json.dumps(result, sort_keys=True))


main()
"""


class EncodedValue(TypedDict):
    kind: str
    value: NotRequired[Json]


class FieldShape(TypedDict):
    name: str
    required: bool
    default: EncodedValue
    description: str | None
    literals: list[Json] | None


class ClassShape(TypedDict):
    kind: str
    bases: list[str]
    config: dict[str, Json]
    fields: list[FieldShape]


class AliasShape(TypedDict):
    literals: list[Json]


class ModuleShape(TypedDict):
    classes: dict[str, ClassShape]
    aliases: dict[str, AliasShape]


class ProbeOutcome(TypedDict):
    outcome: str
    value: EncodedValue


class PackageShape(TypedDict):
    modules: dict[str, ModuleShape]
    probes: list[ProbeOutcome]


class Probe(TypedDict):
    module: str
    target: str
    payload: Json


ESTIMATE_PAYLOAD: dict[str, Json] = {"status": "error", "hours": None, "reasoning": "r"}
METRICS_PAYLOAD: dict[str, Json] = {
    "matched_spend": 1.5,
    "output_hours": 2.5,
    "total_spend": 3.5,
    "total_output_hours": 4.5,
    "excluded_spend": 0.5,
    "cost_per_hour": None,
    "hours_per_dollar": None,
    "merged_prs": 1,
    "estimated_prs": 2,
    "matched_prs": 3,
    "cohort_people": 4,
    "people_with_prs": 5,
    "pending_prs": 6,
}
PULL_PAYLOAD: dict[str, Json] = {
    "repo": "a/b",
    "number": 1,
    "title": "t",
    "url": "u",
    "login": "l",
    "emails": [],
    "profile_email": "",
    "merged_at": "2026-01-01",
    "head_sha": "s",
    "additions": 1,
    "deletions": 2,
    "changed_files": 3,
    "commit_count": 4,
    "incomplete_metadata": False,
    "estimate": ESTIMATE_PAYLOAD,
    "email": "e",
    "match_method": "manual",
    "matched": True,
}
COMPLETION_PAYLOAD: dict[str, Json] = {
    "model": "m",
    "temperature": 0,
    "messages": [{"role": "user", "content": "c"}],
    "response_format": {"type": "json_object"},
    "max_tokens": 1200,
    "metadata": {"tags": ["t"], "litellm_roi_estimator": True},
}
INSIGHT_METRIC_PAYLOAD: dict[str, Json] = {
    "model_group": "g",
    "model": "m",
    "provider": "p",
    "spend": 1.5,
    "prompt_tokens": 1,
    "completion_tokens": 2,
    "requests": 3,
    "successful_requests": 2,
    "failed_requests": 1,
}

PROBES: tuple[Probe, ...] = (
    {"module": ROI_MODULE, "target": "_normalize_login", "payload": "  MixedCase "},
    {"module": ROI_MODULE, "target": "_normalize_login", "payload": "user_name"},
    {"module": ROI_MODULE, "target": "_normalize_login", "payload": "user[name]"},
    {"module": ROI_MODULE, "target": "_normalize_login", "payload": "bad name"},
    {"module": ROI_MODULE, "target": "_normalize_login", "payload": ""},
    {"module": ROI_MODULE, "target": "_normalize_login", "payload": "Üser"},
    {"module": ROI_MODULE, "target": "_normalize_login", "payload": "a-b"},
    {"module": ROI_MODULE, "target": "_normalize_login", "payload": "user.name"},
    {"module": ROI_MODULE, "target": "_normalize_login", "payload": "-"},
    {"module": ROI_MODULE, "target": "ROISettings", "payload": {}},
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"github_api_url": " https://example.com/ "},
    },
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"github_api_url": "http://example.com"},
    },
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"github_api_url": "https://user:pass@example.com"},
    },
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"github_api_url": "https://example.com?q=1"},
    },
    {"module": ROI_MODULE, "target": "ROISettings", "payload": {"github_api_url": ""}},
    {"module": ROI_MODULE, "target": "ROISettings", "payload": {"github_api_url": "https://"}},
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"github_token": "abc", "estimator_key": "def"},
    },
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"repos": ["Owner/Repo.git", "owner/repo/", "owner/repo"]},
    },
    {"module": ROI_MODULE, "target": "ROISettings", "payload": {"repos": ["bad repo"]}},
    {"module": ROI_MODULE, "target": "ROISettings", "payload": {"repos": ["a/../b"]}},
    {"module": ROI_MODULE, "target": "ROISettings", "payload": {"repos": ["a/b", "."]}},
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"estimator_prompt": "  hi  "},
    },
    {"module": ROI_MODULE, "target": "ROISettings", "payload": {"estimator_prompt": "   "}},
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"estimator_prompt": "x" * 20000},
    },
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"estimator_prompt": "x" * 20001},
    },
    {"module": ROI_MODULE, "target": "ROISettings", "payload": {"backfill_days": 1}},
    {"module": ROI_MODULE, "target": "ROISettings", "payload": {"backfill_days": 3650}},
    {"module": ROI_MODULE, "target": "ROISettings", "payload": {"backfill_days": 0}},
    {"module": ROI_MODULE, "target": "ROISettings", "payload": {"backfill_days": 3651}},
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"update_interval_minutes": 0},
    },
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"update_interval_minutes": 4.9},
    },
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"update_interval_minutes": 5},
    },
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"update_interval_minutes": 43200},
    },
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"update_interval_minutes": 43200.5},
    },
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"update_interval_minutes": -1},
    },
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"identity_map": {"User": "User@Example.COM"}},
    },
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"identity_map": {"user": "user@users.noreply.github.com"}},
    },
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"identity_map": {"user": "not-an-email"}},
    },
    {
        "module": ROI_MODULE,
        "target": "ROISettings",
        "payload": {"identity_map": {"bad login": "user@example.com"}},
    },
    {"module": ROI_MODULE, "target": "ROISettings", "payload": {"unknown_key": 1}},
    {"module": ROI_MODULE, "target": "ROISettingsUpdate", "payload": {}},
    {"module": ROI_MODULE, "target": "ROISettingsUpdate", "payload": {"unknown_key": 1}},
    {"module": ROI_MODULE, "target": "ROISettingsUpdate", "payload": {"backfill_days": 0}},
    {
        "module": ROI_MODULE,
        "target": "ROISettingsUpdate",
        "payload": {"update_interval_minutes": 4},
    },
    {"module": ROI_MODULE, "target": "ROISettingsUpdate", "payload": {"repos": ["a/b"]}},
    {
        "module": ROI_MODULE,
        "target": "ROISettingsResponse",
        "payload": {
            "github_api_url": "https://api.github.com",
            "repos": [],
            "estimator_model": "m",
            "estimator_prompt": "p",
            "backfill_days": 7,
            "update_interval_minutes": 1440,
            "has_estimator_key": False,
            "identity_map": {},
            "has_github_token": False,
            "default_prompt": "d",
            "available_models": [],
            "ready": True,
        },
    },
    {
        "module": ROI_MODULE,
        "target": "ROIRepositoriesResponse",
        "payload": {
            "repositories": [{"name": "a/b", "visibility": "private", "archived": False}],
            "page": 1,
            "has_more": False,
        },
    },
    {
        "module": ROI_MODULE,
        "target": "ROISyncStatus",
        "payload": {
            "running": False,
            "phase": "idle",
            "stage": "s",
            "done": 0,
            "total": 0,
            "estimated": 0,
            "reused": 0,
            "needs_attention": 0,
            "error": None,
        },
    },
    {
        "module": ROI_MODULE,
        "target": "ROISyncStatus",
        "payload": {
            "running": False,
            "phase": "bogus",
            "stage": "s",
            "done": 0,
            "total": 0,
            "estimated": 0,
            "reused": 0,
            "needs_attention": 0,
            "error": None,
        },
    },
    {"module": ROI_MODULE, "target": "ROIMetricsResponse", "payload": METRICS_PAYLOAD},
    {
        "module": ROI_MODULE,
        "target": "ROIPersonResponse",
        "payload": {
            "id": "u",
            "email": "e",
            "logins": ["l"],
            "spend": None,
            "hours": 1.0,
            "prs": 2,
            "estimated_prs": 3,
            "pending_prs": 4,
            "match_methods": ["manual"],
            "eligible": True,
            "cost_per_hour": None,
        },
    },
    {
        "module": ROI_MODULE,
        "target": "ROIEstimateResponse",
        "payload": {"status": "estimated", "hours": 1.0, "reasoning": "because"},
    },
    {"module": ROI_MODULE, "target": "ROIPullResponse", "payload": PULL_PAYLOAD},
    {
        "module": ROI_MODULE,
        "target": "ROIPullResponse",
        "payload": {
            **PULL_PAYLOAD,
            "estimate": {"status": "bogus", "hours": None, "reasoning": "r"},
        },
    },
    {
        "module": ROI_MODULE,
        "target": "ROISummaryResponse",
        "payload": {
            "id": None,
            "mode": "m",
            "start": "s",
            "end": "e",
            "synced_at": "t",
            "repos": [],
            "estimator_model": "m",
            "estimator_prompt": "p",
            "warnings": [],
            "effort_basis": None,
            "metrics": METRICS_PAYLOAD,
            "people": [],
            "pulls": [],
            "trend": [{"date": "d", "spend": 1.0, "hours": 2.0, "prs": 3}],
        },
    },
    {"module": ROI_MODULE, "target": "ROIReportResponse", "payload": {"report": None}},
    {
        "module": ROI_MODULE,
        "target": "ROIIdentityMapUpdate",
        "payload": {"github_login": " User ", "email": None},
    },
    {
        "module": ROI_MODULE,
        "target": "ROIIdentityMapUpdate",
        "payload": {"github_login": "bad login", "email": None},
    },
    {
        "module": ROI_MODULE,
        "target": "ROIIdentityMapResponse",
        "payload": {"report": None, "identity_map": {}},
    },
    {
        "module": ROI_MODULE,
        "target": "ROIEstimatorChanges",
        "payload": {"additions": 1, "deletions": 2, "files": 3, "commits": 4},
    },
    {
        "module": ROI_MODULE,
        "target": "ROIEstimatorFile",
        "payload": {"filename": None, "status": None, "additions": None, "deletions": None},
    },
    {
        "module": ROI_MODULE,
        "target": "ROIEstimatorCommit",
        "payload": {"sha": "s", "message": "m"},
    },
    {
        "module": ROI_MODULE,
        "target": "ROIEstimatorEvidence",
        "payload": {
            "repo": "a/b",
            "number": 1,
            "title": "t",
            "body": "b",
            "changes": {"additions": 1, "deletions": 2, "files": 3, "commits": 4},
            "files": [{"filename": "f", "status": "modified", "additions": 1, "deletions": 2}],
            "commits": [{"sha": "s", "message": "m"}],
        },
    },
    {"module": ROI_MODULE, "target": "ROICompletionRequest", "payload": COMPLETION_PAYLOAD},
    {
        "module": ROI_MODULE,
        "target": "ROICompletionRequest",
        "payload": {**COMPLETION_PAYLOAD, "temperature": 1},
    },
    {
        "module": ROI_MODULE,
        "target": "ROICompletionRequest",
        "payload": {**COMPLETION_PAYLOAD, "max_tokens": 1},
    },
    {
        "module": ROI_MODULE,
        "target": "ROICompletionRequest",
        "payload": {**COMPLETION_PAYLOAD, "reasoning_effort": "none"},
    },
    {
        "module": ROI_MODULE,
        "target": "ROICompletionRequest",
        "payload": {**COMPLETION_PAYLOAD, "reasoning_effort": "low"},
    },
    {
        "module": ROI_MODULE,
        "target": "ROICompletionResponse",
        "payload": {"choices": [{"finish_reason": "stop", "message": {"content": "x"}}]},
    },
    {
        "module": ROI_MODULE,
        "target": "ROIEstimatorResult",
        "payload": {"hours": 1, "reasoning": "x"},
    },
    {
        "module": ROI_MODULE,
        "target": "ROIEstimatorResult",
        "payload": {"hours": 1.5, "reasoning": "x"},
    },
    {
        "module": ROI_MODULE,
        "target": "ROIEstimatorResult",
        "payload": {"hours": -1, "reasoning": "x"},
    },
    {
        "module": ROI_MODULE,
        "target": "ROIEstimatorResult",
        "payload": {"hours": True, "reasoning": "x"},
    },
    {
        "module": ROI_MODULE,
        "target": "ROIEstimatorResult",
        "payload": {"hours": 1, "reasoning": "  "},
    },
    {
        "module": ROI_MODULE,
        "target": "ROIEstimatorResult",
        "payload": {"hours": 1, "reasoning": "x", "extra": 1},
    },
    {"module": ROI_MODULE, "target": "ROIEstimatorResult", "payload": {"reasoning": "x"}},
    {"module": MEMORY_MODULE, "target": "LiteLLM_MemoryRow", "payload": {}},
    {
        "module": MEMORY_MODULE,
        "target": "LiteLLM_MemoryRow",
        "payload": {
            "memory_id": "m",
            "key": "k",
            "value": "v",
            "metadata": {"a": [1, True, None]},
            "user_id": "u",
            "team_id": "t",
            "created_at": "2026-01-02T03:04:05Z",
            "created_by": "c",
            "updated_at": "2026-01-02T03:04:05Z",
            "updated_by": "d",
        },
    },
    {
        "module": MEMORY_MODULE,
        "target": "LiteLLM_MemoryRow",
        "payload": {"memory_id": "m", "key": "k", "value": "v", "metadata": 3},
    },
    {
        "module": MEMORY_MODULE,
        "target": "LiteLLM_MemoryRow",
        "payload": {"memory_id": "m", "key": "k", "value": "v", "metadata": [1]},
    },
    {
        "module": MEMORY_MODULE,
        "target": "LiteLLM_MemoryRow",
        "payload": {"memory_id": "m", "key": "k", "value": "v", "metadata": "s"},
    },
    {"module": MEMORY_MODULE, "target": "LiteLLM_MemoryRow", "payload": {"memory_id": "m"}},
    {
        "module": MEMORY_MODULE,
        "target": "MemoryCreateRequest",
        "payload": {"key": "k", "value": "v"},
    },
    {
        "module": MEMORY_MODULE,
        "target": "MemoryCreateRequest",
        "payload": {"key": "k", "value": "v", "metadata": {"x": 1}, "user_id": "u", "team_id": "t"},
    },
    {"module": MEMORY_MODULE, "target": "MemoryCreateRequest", "payload": {"value": "v"}},
    {"module": MEMORY_MODULE, "target": "MemoryUpdateRequest", "payload": {}},
    {
        "module": MEMORY_MODULE,
        "target": "MemoryUpdateRequest",
        "payload": {"value": "v", "metadata": [1, 2], "user_id": "u", "team_id": "t"},
    },
    {
        "module": MEMORY_MODULE,
        "target": "MemoryListResponse",
        "payload": {"memories": [], "total": 0},
    },
    {
        "module": MEMORY_MODULE,
        "target": "MemoryListResponse",
        "payload": {"memories": [{"memory_id": "m", "key": "k", "value": "v"}], "total": 1},
    },
    {
        "module": MEMORY_MODULE,
        "target": "MemoryDeleteResponse",
        "payload": {"key": "k", "deleted": True},
    },
    {"module": MEMORY_MODULE, "target": "MemoryDeleteResponse", "payload": {"key": "k"}},
    {
        "module": INSIGHT_MODULE,
        "target": "ModelInsightMetric",
        "payload": INSIGHT_METRIC_PAYLOAD,
    },
    {
        "module": INSIGHT_MODULE,
        "target": "ModelInsightMetric",
        "payload": {"model_group": "g"},
    },
    {
        "module": INSIGHT_MODULE,
        "target": "ModelInsightDailyMetric",
        "payload": {**INSIGHT_METRIC_PAYLOAD, "date": "2026-01-01"},
    },
    {
        "module": INSIGHT_MODULE,
        "target": "ModelInsightDailyMetric",
        "payload": INSIGHT_METRIC_PAYLOAD,
    },
    {
        "module": INSIGHT_MODULE,
        "target": "ModelInsightDailyTotal",
        "payload": {
            "date": "2026-01-01",
            "spend": 1.5,
            "prompt_tokens": 1,
            "completion_tokens": 2,
            "requests": 3,
        },
    },
    {
        "module": INSIGHT_MODULE,
        "target": "ModelInsightTask",
        "payload": {"task_type": "t", "label": "l", "category": "c"},
    },
    {
        "module": INSIGHT_MODULE,
        "target": "ModelInsightTaskSummary",
        "payload": {
            "task_type": "t",
            "label": "l",
            "category": "c",
            "value": 1.5,
            "share": 0.5,
            "leader": "l",
            "provider": "p",
        },
    },
    {
        "module": INSIGHT_MODULE,
        "target": "ModelInsightTaskSummary",
        "payload": {"task_type": "t", "label": "l", "category": "c", "value": 1.5, "share": 0.5},
    },
    {
        "module": INSIGHT_MODULE,
        "target": "ModelInsightsResponse",
        "payload": {
            "start_date": "2026-01-01",
            "end_date": "2026-01-02",
            "daily": [{**INSIGHT_METRIC_PAYLOAD, "date": "2026-01-01"}],
            "daily_totals": [
                {
                    "date": "2026-01-01",
                    "spend": 1.5,
                    "prompt_tokens": 1,
                    "completion_tokens": 2,
                    "requests": 3,
                }
            ],
            "top_models": [INSIGHT_METRIC_PAYLOAD],
        },
    },
    {
        "module": INSIGHT_MODULE,
        "target": "ModelInsightTasksResponse",
        "payload": {
            "start_date": "2026-01-01",
            "end_date": "2026-01-02",
            "tasks": [
                {
                    "task_type": "t",
                    "label": "l",
                    "category": "c",
                    "value": 1.5,
                    "share": 0.5,
                    "leader": "l",
                    "provider": "p",
                }
            ],
        },
    },
)


def describe_package(package: str, probes: tuple[Probe, ...]) -> PackageShape:
    completed = subprocess.run(
        [sys.executable, "-c", DESCRIBE_SCRIPT, package],
        input=json.dumps(probes),
        capture_output=True,
        text=True,
        check=False,
        timeout=DESCRIBE_TIMEOUT_SECONDS,
    )
    assert completed.returncode == 0, completed.stderr
    return TypeAdapter(PackageShape).validate_json(completed.stdout)


@pytest.fixture(scope="module")
def package_shapes() -> tuple[PackageShape, PackageShape]:
    ours = describe_package("betterlite", PROBES)
    theirs = describe_package("litellm", PROBES)
    return ours, theirs


def test_module_names_match(package_shapes: tuple[PackageShape, PackageShape]) -> None:
    ours, theirs = package_shapes
    assert ours["modules"] == theirs["modules"]


def test_behavior_names_match(package_shapes: tuple[PackageShape, PackageShape]) -> None:
    ours, theirs = package_shapes
    for index, (left, right) in enumerate(zip(ours["probes"], theirs["probes"], strict=True)):
        assert left == right, index
