import asyncio
import importlib
import random
import re
import time
from collections.abc import Iterator, Mapping
from datetime import datetime
from typing import Final, Protocol

import pytest
from litellm.exceptions import ModelNotMappedError as LiteLLMModelNotMapped
from litellm.utils import (
    get_max_tokens as litellm_get_max_tokens,
)
from litellm.utils import (
    get_provider_info as litellm_get_provider_info,
)
from model_map_server import (
    CORRUPT_PATH,
    GOOD_PATH,
    RETRY_BETTERLITE_PATH,
    RETRY_LITELLM_PATH,
    ModelMapServer,
    bundled_body,
)
from pydantic import JsonValue, TypeAdapter

import betterlite
from betterlite import (
    ModelCostMapReloaded,
    ModelCostMapReloadUnavailable,
    adopt_model_cost_map,
    get_max_tokens,
    get_model_cost_map,
    get_model_cost_map_loaded_at,
    get_model_cost_map_provenance,
    get_model_cost_map_source_info,
    get_model_cost_mutation_generation,
    get_model_info,
    get_provider_info,
    git_blob_id,
    refetch_model_cost_map,
)
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite.exceptions import ModelNotMappedError

LOCAL_MAP_ENV: Final = "LITELLM_LOCAL_MODEL_COST_MAP"
UNMAPPED_MODEL: Final = "not-a-real-model-xyz"
BEDROCK_MODEL: Final = "bedrock/us.anthropic.claude-3-5-haiku-20241022-v1:0"
VERTEX_MODEL: Final = "vertex_ai/gemini-3-pro-preview"
FINE_TUNE_MODEL: Final = "ft:gpt-4o-2024-08-06"
FIREWORKS_MODEL: Final = "fireworks_ai/accounts/fireworks/models/llama-v3p1-8b-instruct"
FIXED_MODELS: Final = ("gpt-4", BEDROCK_MODEL, VERTEX_MODEL, FINE_TUNE_MODEL)
REASONING_MODELS: Final = ("gpt-5", "o3", "openai/gpt-5", "openai/o3")
PARAMS_DIFFER: Final = frozenset({BEDROCK_MODEL, VERTEX_MODEL, FINE_TUNE_MODEL})
UNSUPPORTED_PARAMS: Final = "supported_openai_params"
PROBE_MAX_TOKENS: Final = 77
REMOVAL_MAX_TOKENS: Final = 3
ADOPT_MAX_TOKENS: Final = 5
_MODEL_INFO: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
_PARAMETER_NAMES: Final[TypeAdapter[list[str]]] = TypeAdapter(list[str])
_TEXT: Final[TypeAdapter[str]] = TypeAdapter(str)
COST_MAP_MODULE: Final = "litellm.litellm_core_utils.get_model_cost_map"


class ReferenceCall(Protocol):
    def __call__(self, *args: JsonValue, **kwargs: JsonValue) -> JsonValue: ...


class ReferenceModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


class LookupCall(Protocol):
    def __call__(self, *args: JsonValue, **kwargs: JsonValue) -> JsonValue: ...

    def cache_clear(self) -> None: ...


class LookupModule(Protocol):
    def __getattr__(self, name: str) -> LookupCall: ...


class ModelCostModule(Protocol):
    def __getattr__(self, name: str) -> dict[str, JsonValue]: ...


class InfoCall(Protocol):
    def __call__(self) -> dict[str, JsonValue]: ...


class InfoModule(Protocol):
    def __getattr__(self, name: str) -> InfoCall: ...


class LiteReloadView(Protocol):
    @property
    def model_cost_map(self) -> dict[str, JsonValue]: ...

    @property
    def revision(self) -> str | None: ...

    @property
    def etag(self) -> str | None: ...

    @property
    def reason(self) -> str: ...


class RefetchCall(Protocol):
    async def __call__(self, url: str) -> LiteReloadView: ...


class RefetchModule(Protocol):
    def __getattr__(self, name: str) -> RefetchCall: ...


REFERENCE: ReferenceModule = importlib.import_module("litellm")
COST_MAP: ReferenceModule = importlib.import_module(COST_MAP_MODULE)
INFO: InfoModule = importlib.import_module(COST_MAP_MODULE)
REFETCH: RefetchModule = importlib.import_module(COST_MAP_MODULE)
LOOKUP: LookupModule = importlib.import_module("litellm")
REFERENCE_COST: ModelCostModule = importlib.import_module("litellm")


def without_params(info: Mapping[str, JsonValue]) -> dict[str, JsonValue]:
    copy = dict(info)
    copy.pop(UNSUPPORTED_PARAMS, None)
    return copy


class _OtherSleeper:
    def __init__(self) -> None:
        self.seconds = 0.0

    def __call__(self, seconds: float) -> None:
        self.seconds = seconds


@pytest.fixture(scope="module")
def server() -> Iterator[ModelMapServer]:
    with ModelMapServer() as running:
        yield running


def assert_source_info_matches() -> None:
    ours = get_model_cost_map_source_info()
    theirs = INFO.get_model_cost_map_source_info()
    assert set(ours) == set(theirs)
    assert ours["source"] == theirs["source"]
    assert ours["url"] == theirs["url"]
    assert ours["is_env_forced"] == theirs["is_env_forced"]
    assert ours["source_revision"] == theirs["source_revision"]
    assert ours["etag"] == theirs["etag"]
    assert ours["loaded_at"] is not None
    assert theirs["loaded_at"] is not None


def test_lookup_fixed_models() -> None:
    for model in FIXED_MODELS:
        theirs = _MODEL_INFO.validate_python(REFERENCE.get_model_info(model))
        ours = get_model_info(model)
        if model in PARAMS_DIFFER:
            assert without_params(ours) == without_params(theirs)
        else:
            assert ours == theirs


def test_lookup_reasoning_models() -> None:
    for model in REASONING_MODELS:
        theirs = _MODEL_INFO.validate_python(REFERENCE.get_model_info(model))
        assert get_model_info(model) == theirs


def test_lookup_unmapped_model() -> None:
    with pytest.raises(LiteLLMModelNotMapped) as theirs:
        REFERENCE.get_model_info(UNMAPPED_MODEL)
    with pytest.raises(ModelNotMappedError) as ours:
        get_model_info(UNMAPPED_MODEL)
    assert str(ours.value) == str(theirs.value)


def test_lookup_max_tokens() -> None:
    for model in ("gpt-4", "gpt-4o", FINE_TUNE_MODEL):
        assert get_max_tokens(model) == litellm_get_max_tokens(model)
    with pytest.raises(Exception, match=re.escape("isn't mapped yet")) as theirs:
        litellm_get_max_tokens(UNMAPPED_MODEL)
    with pytest.raises(Exception, match=re.escape("isn't mapped yet")) as ours:
        get_max_tokens(UNMAPPED_MODEL)
    assert str(ours.value) == str(theirs.value)


def test_lookup_provider_info() -> None:
    for model, provider in (("grok-3", "xai"), (FIREWORKS_MODEL, "fireworks_ai")):
        theirs = litellm_get_provider_info(model, provider)
        assert theirs is not None
        assert get_provider_info(model, provider) == _MODEL_INFO.validate_python(theirs)
    assert get_provider_info("gpt-4", "openai") is None
    assert litellm_get_provider_info("gpt-4", "openai") is None


def test_lookup_cache_attributes() -> None:
    get_model_info.cache_clear()
    get_model_info("gpt-4")
    info = get_model_info.cache_info()
    assert info.hits == 0
    assert info.misses == 1


def test_lookup_api_key_matches() -> None:
    theirs = _MODEL_INFO.validate_python(
        REFERENCE.get_model_info("gpt-4", api_key="sk-test")
    )
    ours = get_model_info("gpt-4", api_key="sk-test")
    assert ours == theirs


def test_sync_model_cost_change() -> None:
    entry: dict[str, JsonValue] = {"litellm_provider": "openai", "max_tokens": PROBE_MAX_TOKENS}
    betterlite.model_cost["sync-probe-model"] = entry
    try:
        ours = get_model_info("sync-probe-model")
        assert ours["max_tokens"] == PROBE_MAX_TOKENS
        assert get_max_tokens("sync-probe-model") == PROBE_MAX_TOKENS
    finally:
        del betterlite.model_cost["sync-probe-model"]


def test_sync_removed_entry() -> None:
    betterlite.model_cost["sync-removal-probe"] = {
        "litellm_provider": "openai",
        "max_tokens": REMOVAL_MAX_TOKENS,
    }
    try:
        assert get_max_tokens("sync-removal-probe") == REMOVAL_MAX_TOKENS
    finally:
        del betterlite.model_cost["sync-removal-probe"]
    with pytest.raises(ModelNotMappedError):
        get_max_tokens("sync-removal-probe")


def test_sync_generation() -> None:
    start = get_model_cost_mutation_generation()
    get_model_info("gpt-4")
    assert get_model_cost_mutation_generation() == start
    betterlite.model_cost["sync-generation-probe"] = {"litellm_provider": "openai"}
    try:
        get_model_info("sync-generation-probe")
        moved = get_model_cost_mutation_generation()
        assert moved > start
        get_model_info("sync-generation-probe")
        assert get_model_cost_mutation_generation() == moved
    finally:
        del betterlite.model_cost["sync-generation-probe"]
        get_model_info("gpt-4")


def test_sync_capability_flags() -> None:
    model = "o3"
    ours_original = betterlite.model_cost[model]
    theirs_original = REFERENCE_COST.model_cost[model]
    ours_entry = dict(_MODEL_INFO.validate_python(ours_original))
    theirs_entry = dict(_MODEL_INFO.validate_python(theirs_original))
    ours_entry["supports_function_calling"] = False
    theirs_entry["supports_function_calling"] = False
    betterlite.model_cost[model] = ours_entry
    REFERENCE_COST.model_cost[model] = theirs_entry
    LOOKUP.get_model_info.cache_clear()
    try:
        theirs = _MODEL_INFO.validate_python(LOOKUP.get_model_info(model))
        ours = get_model_info(model)
        assert ours == theirs
        assert ours["supports_function_calling"] is False
        parameters = _PARAMETER_NAMES.validate_python(ours[UNSUPPORTED_PARAMS])
        assert "tools" not in parameters
        assert "tool_choice" not in parameters
        assert "function_call" not in parameters
        assert "functions" not in parameters
        assert "reasoning_effort" in parameters
    finally:
        betterlite.model_cost[model] = ours_original
        REFERENCE_COST.model_cost[model] = theirs_original
        LOOKUP.get_model_info.cache_clear()
        get_model_info("gpt-4")


def test_sync_prefixed_flags() -> None:
    model = "o3"
    prefixed = "openai/o3"
    ours_entry = dict(_MODEL_INFO.validate_python(betterlite.model_cost[model]))
    theirs_entry = dict(_MODEL_INFO.validate_python(REFERENCE_COST.model_cost[model]))
    ours_entry["supports_function_calling"] = False
    theirs_entry["supports_function_calling"] = False
    betterlite.model_cost[prefixed] = ours_entry
    REFERENCE_COST.model_cost[prefixed] = theirs_entry
    LOOKUP.get_model_info.cache_clear()
    try:
        prefixed_theirs = _MODEL_INFO.validate_python(LOOKUP.get_model_info(prefixed))
        prefixed_ours = get_model_info(prefixed)
        assert prefixed_ours == prefixed_theirs
        assert prefixed_ours["supports_function_calling"] is False
        prefixed_parameters = _PARAMETER_NAMES.validate_python(
            prefixed_ours[UNSUPPORTED_PARAMS]
        )
        assert "tools" not in prefixed_parameters
        assert "tool_choice" not in prefixed_parameters
        assert "function_call" not in prefixed_parameters
        assert "functions" not in prefixed_parameters
        assert "reasoning_effort" in prefixed_parameters
        bare_theirs = _MODEL_INFO.validate_python(LOOKUP.get_model_info(model))
        bare_ours = get_model_info(model)
        assert bare_ours == bare_theirs
        bare_parameters = _PARAMETER_NAMES.validate_python(bare_ours[UNSUPPORTED_PARAMS])
        assert "tools" not in bare_parameters
        assert "reasoning_effort" in bare_parameters
    finally:
        del betterlite.model_cost[prefixed]
        del REFERENCE_COST.model_cost[prefixed]
        LOOKUP.get_model_info.cache_clear()
        get_model_info("gpt-4")


def test_reporter_local_shapes(
    server: ModelMapServer, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv(LOCAL_MAP_ENV, "true")
    _ = REFERENCE.get_model_cost_map(server.url + GOOD_PATH)
    get_model_cost_map(server.url + GOOD_PATH)
    assert_source_info_matches()
    assert get_model_cost_map_provenance() == INFO.get_model_cost_map_provenance()
    info = get_model_cost_map_source_info()
    assert info["source"] == "local"
    assert info["url"] is None
    assert info["is_env_forced"] is True
    assert info["fallback_reason"] is None
    assert info["etag"] is None
    assert info["source_revision"] == git_blob_id(bundled_body())
    assert get_model_cost_map_loaded_at() is not None
    assert COST_MAP.get_model_cost_map_loaded_at() is not None


def test_reporter_loaded_at_micros() -> None:
    text = _TEXT.validate_python(get_model_cost_map_source_info()["loaded_at"])
    assert re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{6}\+00:00", text)
    assert datetime.fromisoformat(text).utcoffset() is not None


def test_reload_good_body(
    server: ModelMapServer, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv(LOCAL_MAP_ENV, raising=False)
    url = server.url + GOOD_PATH
    theirs = _MODEL_INFO.validate_python(REFERENCE.get_model_cost_map(url))
    ours = get_model_cost_map(url, 5, 3)
    assert ours == theirs
    info = get_model_cost_map_source_info()
    assert info["source"] == "remote"
    assert info["url"] == url
    assert info["is_env_forced"] is False
    assert info["fallback_reason"] is None
    assert info["etag"] == "etag" + GOOD_PATH


def test_reload_corrupt_body(
    server: ModelMapServer, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv(LOCAL_MAP_ENV, raising=False)
    url = server.url + CORRUPT_PATH
    theirs = _MODEL_INFO.validate_python(REFERENCE.get_model_cost_map(url))
    ours = get_model_cost_map(url)
    assert ours == theirs
    our_reason = get_model_cost_map_source_info()["fallback_reason"]
    their_info = INFO.get_model_cost_map_source_info()
    their_reason = _TEXT.validate_python(their_info["fallback_reason"])
    assert our_reason is not None
    assert our_reason.startswith(f"Remote fetch failed: invalid JSON from {url}")
    assert their_reason.startswith(f"Remote fetch failed: invalid JSON from {url}")
    their_reload = asyncio.run(REFETCH.refetch_model_cost_map(url))
    our_reload = asyncio.run(refetch_model_cost_map(url))
    match our_reload:
        case ModelCostMapReloadUnavailable():
            assert our_reload.reason.startswith(f"invalid JSON from {url}")
            assert their_reload.reason.startswith(f"invalid JSON from {url}")
        case ModelCostMapReloaded():
            pytest.fail("our refetch reloaded a corrupt map")


def test_reload_retry_then_good(
    server: ModelMapServer, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv(LOCAL_MAP_ENV, raising=False)
    their_url = server.url + RETRY_LITELLM_PATH
    our_url = server.url + RETRY_BETTERLITE_PATH
    theirs = asyncio.run(REFETCH.refetch_model_cost_map(their_url))
    ours = asyncio.run(refetch_model_cost_map(our_url, 5, 3))
    match ours:
        case ModelCostMapReloaded():
            assert theirs.model_cost_map == ours.model_cost_map
            assert theirs.revision == ours.revision
            assert ours.etag == f"etag{RETRY_BETTERLITE_PATH}"
            assert theirs.etag == f"etag{RETRY_LITELLM_PATH}"
        case ModelCostMapReloadUnavailable():
            pytest.fail("our refetch answered unavailable")
    our_info = get_model_cost_map_source_info()
    their_info = INFO.get_model_cost_map_source_info()
    assert set(our_info) == set(their_info)
    assert our_info["source"] == their_info["source"] == "remote"
    assert our_info["is_env_forced"] is False
    assert our_info["fallback_reason"] is None
    assert their_info["fallback_reason"] is None
    assert our_info["source_revision"] == their_info["source_revision"]
    assert our_info["loaded_at"] is not None
    assert their_info["loaded_at"] is not None


def test_unsupported_options(server: ModelMapServer) -> None:
    url = server.url + GOOD_PATH
    with pytest.raises(UnsupportedCallArgument):
        get_model_cost_map(url, client=object())
    with pytest.raises(UnsupportedCallArgument):
        get_model_cost_map(url, sleep=_OtherSleeper())
    with pytest.raises(UnsupportedCallArgument):
        get_model_cost_map(url, 5, 3, time.sleep, random.SystemRandom(), object())
    with pytest.raises(UnsupportedCallArgument):
        asyncio.run(refetch_model_cost_map(url, rng=random.SystemRandom()))
    with pytest.raises(UnsupportedCallArgument):
        asyncio.run(
            refetch_model_cost_map(url, 5, 3, asyncio.sleep, random.SystemRandom(), object())
        )


def test_reload_adopt_cost_map(
    server: ModelMapServer, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv(LOCAL_MAP_ENV, raising=False)
    loaded = REFERENCE.get_model_cost_map(server.url + GOOD_PATH)
    their_backup = _MODEL_INFO.validate_python(loaded)
    our_backup = betterlite.model_cost
    adopted: dict[str, JsonValue] = {
        "adopt-probe": {"litellm_provider": "openai", "max_tokens": ADOPT_MAX_TOKENS}
    }
    try:
        generation = get_model_cost_mutation_generation()
        ours_count = adopt_model_cost_map(adopted)
        theirs_count = COST_MAP.adopt_model_cost_map(adopted)
        assert ours_count == theirs_count == 1
        assert get_model_info("adopt-probe")["max_tokens"] == ADOPT_MAX_TOKENS
        assert get_model_cost_mutation_generation() > generation
        empty: dict[str, JsonValue] = {}
        assert adopt_model_cost_map(empty) == 0
        assert COST_MAP.adopt_model_cost_map(empty) == 0
    finally:
        adopt_model_cost_map(our_backup)
        COST_MAP.adopt_model_cost_map(their_backup)
        get_model_info("gpt-4")
