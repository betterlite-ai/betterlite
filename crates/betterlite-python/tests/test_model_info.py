import asyncio
import random
import re
from collections.abc import Iterator, Mapping
from typing import Final

import litellm
import pytest
from litellm.exceptions import ModelNotMappedError as LiteLLMModelNotMapped
from litellm.litellm_core_utils.get_model_cost_map import (
    ModelCostMapReloaded as LiteModelCostMapReloaded,
    ModelCostMapReloadUnavailable as LiteModelCostMapReloadUnavailable,
    adopt_model_cost_map as litellm_adopt_model_cost_map,
    get_model_cost_map_loaded_at as litellm_loaded_at,
    get_model_cost_map_provenance as litellm_provenance,
    get_model_cost_map_source_info as litellm_source_info,
    refetch_model_cost_map as litellm_refetch,
)
from litellm.utils import (
    get_max_tokens as litellm_get_max_tokens,
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
PARAMS_DIFFER: Final = frozenset({BEDROCK_MODEL, VERTEX_MODEL, FINE_TUNE_MODEL})
UNSUPPORTED_PARAMS: Final = "supported_openai_params"
PROBE_MAX_TOKENS: Final = 77
REMOVAL_MAX_TOKENS: Final = 3
ADOPT_MAX_TOKENS: Final = 5
_MODEL_INFO: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


def fields_of(info: Mapping[str, JsonValue]) -> dict[str, JsonValue]:
    copy = dict(info)
    return copy


def without_params(info: Mapping[str, JsonValue]) -> dict[str, JsonValue]:
    copy = fields_of(info)
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
    theirs = litellm_source_info()
    assert set(ours) == set(theirs)
    assert ours["source"] == theirs["source"]
    assert ours["url"] == theirs["url"]
    assert ours["is_env_forced"] == theirs["is_env_forced"]
    assert ours["source_revision"] == theirs["source_revision"]
    assert ours["etag"] == theirs["etag"]
    assert ours["loaded_at"] is not None
    assert theirs["loaded_at"] is not None


def assert_provenance_matches() -> None:
    assert get_model_cost_map_provenance() == litellm_provenance()


def test_lookup_fixed_models() -> None:
    for model in FIXED_MODELS:
        theirs = _MODEL_INFO.validate_python(litellm.get_model_info(model))
        ours = get_model_info(model)
        if model in PARAMS_DIFFER:
            assert without_params(ours) == without_params(theirs)
        else:
            assert ours == theirs


def test_lookup_unmapped_model() -> None:
    with pytest.raises(LiteLLMModelNotMapped) as theirs:
        litellm.get_model_info(UNMAPPED_MODEL)
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
        assert get_provider_info(model, provider) == litellm_get_provider_info(model, provider)
    assert get_provider_info("gpt-4", "openai") is None
    assert litellm_get_provider_info("gpt-4", "openai") is None


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


def test_reporter_local_shapes(
    server: ModelMapServer, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv(LOCAL_MAP_ENV, "true")
    litellm.get_model_cost_map(server.url + GOOD_PATH)
    get_model_cost_map(server.url + GOOD_PATH)
    assert_source_info_matches()
    assert_provenance_matches()
    info = get_model_cost_map_source_info()
    assert info["source"] == "local"
    assert info["url"] is None
    assert info["is_env_forced"] is True
    assert info["fallback_reason"] is None
    assert info["etag"] is None
    assert info["source_revision"] == git_blob_id(bundled_body())
    assert get_model_cost_map_loaded_at() is not None
    assert litellm_loaded_at() is not None


def test_reload_good_body(
    server: ModelMapServer, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv(LOCAL_MAP_ENV, raising=False)
    url = server.url + GOOD_PATH
    theirs = _MODEL_INFO.validate_python(litellm.get_model_cost_map(url))
    ours = get_model_cost_map(url)
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
    theirs = _MODEL_INFO.validate_python(litellm.get_model_cost_map(url))
    ours = get_model_cost_map(url)
    assert ours == theirs
    our_reason = get_model_cost_map_source_info()["fallback_reason"]
    their_reason = litellm_source_info()["fallback_reason"]
    assert our_reason is not None
    assert their_reason is not None
    assert our_reason.startswith(f"Remote fetch failed: invalid JSON from {url}")
    assert their_reason.startswith(f"Remote fetch failed: invalid JSON from {url}")


def test_reload_retry_then_good(
    server: ModelMapServer, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv(LOCAL_MAP_ENV, raising=False)
    their_url = server.url + RETRY_LITELLM_PATH
    our_url = server.url + RETRY_BETTERLITE_PATH
    theirs = asyncio.run(litellm_refetch(their_url))
    ours = asyncio.run(refetch_model_cost_map(our_url))
    matched = False
    match (ours, theirs):
        case (ModelCostMapReloaded(), LiteModelCostMapReloaded()):
            matched = True
            assert ours.model_cost_map == theirs.model_cost_map
            assert ours.revision == theirs.revision
            assert ours.etag == f"etag{RETRY_BETTERLITE_PATH}"
            assert theirs.etag == f"etag{RETRY_LITELLM_PATH}"
    assert matched
    our_info = get_model_cost_map_source_info()
    their_info = litellm_source_info()
    assert set(our_info) == set(their_info)
    assert our_info["source"] == their_info["source"] == "remote"
    assert our_info["is_env_forced"] is False
    assert our_info["fallback_reason"] is None
    assert their_info["fallback_reason"] is None
    assert our_info["source_revision"] == their_info["source_revision"]
    assert our_info["loaded_at"] is not None
    assert their_info["loaded_at"] is not None


def test_reload_refetch_corrupt(
    server: ModelMapServer, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv(LOCAL_MAP_ENV, raising=False)
    url = server.url + CORRUPT_PATH
    theirs = asyncio.run(litellm_refetch(url))
    ours = asyncio.run(refetch_model_cost_map(url))
    matched = False
    match (ours, theirs):
        case (ModelCostMapReloadUnavailable(), LiteModelCostMapReloadUnavailable()):
            matched = True
            assert ours.reason.startswith(f"invalid JSON from {url}")
            assert theirs.reason.startswith(f"invalid JSON from {url}")
    assert matched


def test_unsupported_options(server: ModelMapServer) -> None:
    url = server.url + GOOD_PATH
    with pytest.raises(UnsupportedCallArgument):
        get_model_cost_map(url, client=object())
    with pytest.raises(UnsupportedCallArgument):
        get_model_cost_map(url, sleep=_OtherSleeper())
    with pytest.raises(UnsupportedCallArgument):
        asyncio.run(refetch_model_cost_map(url, rng=random.SystemRandom()))


def test_reload_adopt_cost_map(
    server: ModelMapServer, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv(LOCAL_MAP_ENV, raising=False)
    their_backup = _MODEL_INFO.validate_python(litellm.get_model_cost_map(server.url + GOOD_PATH))
    our_backup = betterlite.model_cost
    adopted: dict[str, JsonValue] = {
        "adopt-probe": {"litellm_provider": "openai", "max_tokens": ADOPT_MAX_TOKENS}
    }
    try:
        generation = get_model_cost_mutation_generation()
        ours_count = adopt_model_cost_map(adopted)
        theirs_count = litellm_adopt_model_cost_map(adopted)
        assert ours_count == theirs_count == 1
        assert get_model_info("adopt-probe")["max_tokens"] == ADOPT_MAX_TOKENS
        assert get_model_cost_mutation_generation() > generation
        assert adopt_model_cost_map({}) == 0
        assert litellm_adopt_model_cost_map({}) == 0
    finally:
        adopt_model_cost_map(our_backup)
        litellm_adopt_model_cost_map(their_backup)
        get_model_info("gpt-4")
