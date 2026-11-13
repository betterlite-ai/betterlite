import importlib
import inspect
import json
import re
import tomllib
from importlib import metadata
from pathlib import Path
from typing import TypedDict

import litellm
import pytest
from packaging.requirements import Requirement
from packaging.utils import canonicalize_name
from pydantic import TypeAdapter

import betterlite
from betterlite import _native
from betterlite._version import installed_version
from betterlite.types.utils import JsonValue as Json

CRATE = Path(__file__).resolve().parents[1]
ROOT = CRATE.parents[1]
PACKAGE = Path(inspect.getfile(betterlite)).parent

MIN_COST_ENTRIES = 1000
MIN_SET_COUNT = 60
MIN_CHAT_MODELS = 100

FAMILIES = (
    "a2a",
    "assistant_calls",
    "assistant_runs",
    "audio",
    "batch_calls",
    "batch_listing",
    "cache",
    "cache_backends",
    "completion",
    "container_calls",
    "container_files",
    "cost",
    "cost_extra",
    "decision_calls",
    "embeddings",
    "eval_calls",
    "eval_runs",
    "file_calls",
    "file_content",
    "genai",
    "guardrail_call",
    "images",
    "interactions",
    "mcp",
    "messages",
    "model_info",
    "moderation",
    "passthrough_calls",
    "provider",
    "proxy_client",
    "rag_calls",
    "realtime_calls",
    "rerank",
    "response_lifecycle",
    "responses",
    "router_call",
    "search_ocr",
    "skill_calls",
    "tokens",
    "tuning_calls",
    "vector_files",
    "vector_manage",
    "vector_search",
    "video_calls",
    "video_edits",
)

PYTHON_MODULES = (
    "_tokens",
    "_model_info",
    "_provider",
    "_cost",
    "_cost_extra",
    "_cache_ops",
    "caching",
    "budget_manager",
    "utils",
)


class CargoPackage(TypedDict):
    version: str


class CargoWorkspace(TypedDict):
    package: CargoPackage


class CargoToml(TypedDict):
    workspace: CargoWorkspace


class LockPackage(TypedDict):
    name: str
    version: str


class LockFile(TypedDict):
    package: list[LockPackage]


def crate_version() -> str:
    parsed: dict[str, Json] = tomllib.loads((ROOT / "Cargo.toml").read_text())
    return TypeAdapter(CargoToml).validate_python(parsed)["workspace"]["package"]["version"]


def test_version_is_crate_version() -> None:
    expected = crate_version()
    assert _native.core_version() == expected
    assert betterlite.__version__ == expected
    assert metadata.version("betterlite") == expected


def test_version_falls_back() -> None:
    assert installed_version("betterlite") == betterlite.__version__
    assert installed_version("betterlite-not-installed") == "unknown"


def test_wheel_is_abi3_cp310() -> None:
    wheel = metadata.distribution("betterlite").read_text("WHEEL")
    assert wheel is not None
    assert re.search(r"^Tag: cp310-abi3-", wheel, re.MULTILINE)


def test_requires_match_lock() -> None:
    lock_text = (ROOT / "tools/oracle/litellm/uv.lock").read_text()
    lock_data: dict[str, Json] = tomllib.loads(lock_text)
    locked = {
        canonicalize_name(package["name"]): package["version"]
        for package in TypeAdapter(LockFile).validate_python(lock_data)["package"]
    }
    requires = metadata.distribution("betterlite").requires
    assert requires is not None
    declared = {
        canonicalize_name(parsed.name): str(parsed.specifier)
        for parsed in map(Requirement, requires)
    }
    expected = {
        canonicalize_name(name): f"=={locked[canonicalize_name(name)]}"
        for name in ("pydantic", "python-dotenv", "typing-extensions")
    }
    assert declared == expected


def test_model_cost_is_litellms() -> None:
    bundled: dict[str, Json] = json.loads(_native.bundled_model_cost_json())
    del bundled["fallback_generalizations"]
    loaded: dict[str, Json] = litellm.model_cost  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563
    assert bundled == loaded
    assert len(loaded) > MIN_COST_ENTRIES


@pytest.fixture(scope="module")
def litellm_sets() -> dict[str, list[str]]:
    members: list[tuple[str, set[str] | list[str]]] = inspect.getmembers(litellm)
    named = dict(members)
    for name in _native.provider_model_sets():
        named[name].clear()
    litellm.add_known_models()  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L1009
    return {name: sorted(set(named[name])) for name in _native.provider_model_sets()}


def test_model_sets_match_litellm(litellm_sets: dict[str, list[str]]) -> None:
    sets = _native.provider_model_sets()
    assert len(sets) > MIN_SET_COUNT
    assert len(sets["open_ai_chat_completion_models"]) > MIN_CHAT_MODELS
    assert sets == litellm_sets


def test_families_are_listed() -> None:
    lib = (CRATE / "src/lib.rs").read_text()
    declared: list[str] = re.findall(r"^pub mod (\w+);$", lib, re.MULTILINE)
    registered: list[str] = re.findall(r"^    (\w+)::register,$", lib, re.MULTILINE)
    stubs = (PACKAGE / "_native.pyi").read_text()
    starred: list[str] = re.findall(
        r"^from betterlite\._native_(\w+) import \*$", stubs, re.MULTILINE
    )
    assert declared == list(FAMILIES)
    assert registered == list(FAMILIES)
    assert starred == list(FAMILIES)


@pytest.mark.parametrize("family", FAMILIES)
def test_family_stub_is_empty(family: str) -> None:
    stub = PACKAGE / f"_native_{family}.pyi"
    assert stub.read_text() == ""
    assert (CRATE / "src" / f"{family}.rs").exists()


def test_init_lists_python_modules() -> None:
    text = (PACKAGE / "__init__.py").read_text()
    starred: list[str] = re.findall(r"^from betterlite\.(\w+) import \*", text, re.MULTILINE)
    assert sorted(starred) == sorted(PYTHON_MODULES)


@pytest.mark.parametrize("name", PYTHON_MODULES)
def test_python_module_is_empty(name: str) -> None:
    module = importlib.import_module(f"betterlite.{name}")
    public = [member for member in dir(module) if not member.startswith("_")]
    assert public == []


def test_caching_is_a_package() -> None:
    module = importlib.import_module("betterlite.caching")
    assert list(module.__path__)
