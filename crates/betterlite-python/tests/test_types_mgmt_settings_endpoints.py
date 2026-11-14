from collections.abc import Sequence
from typing import get_args

import litellm.types.management_endpoints.cache_settings_endpoints as litellm_cache
import litellm.types.management_endpoints.coordination_redis_endpoints as litellm_coordination
import litellm.types.management_endpoints.router_settings_endpoints as litellm_router
import pydantic
import pytest

import betterlite.types.management_endpoints.cache_settings_endpoints as betterlite_cache
import betterlite.types.management_endpoints.coordination_redis_endpoints as betterlite_coordination
import betterlite.types.management_endpoints.router_settings_endpoints as betterlite_router
from betterlite.types.utils import JsonValue


def required_map(model: type[pydantic.BaseModel]) -> dict[str, bool]:
    required: dict[str, bool] = {}
    for name, field in model.model_fields.items():
        required[name] = field.is_required()
    return required


def default_map(model: type[pydantic.BaseModel]) -> dict[str, JsonValue]:
    defaults: dict[str, JsonValue] = {}
    for name, field in model.model_fields.items():
        default: JsonValue = field.default
        defaults[name] = default
    return defaults


def model_dumps(entries: Sequence[pydantic.BaseModel]) -> list[dict[str, JsonValue]]:
    dumped: list[dict[str, JsonValue]] = []
    for entry in entries:
        item: dict[str, JsonValue] = entry.model_dump()
        dumped.append(item)
    return dumped


def test_cache_model_names() -> None:
    ours = betterlite_cache.CacheSettingsField
    reference = litellm_cache.CacheSettingsField
    assert list(ours.model_fields) == list(reference.model_fields)
    assert required_map(ours) == required_map(reference)
    assert default_map(ours) == default_map(reference)


def test_cache_field_names() -> None:
    ours = betterlite_cache.CACHE_SETTINGS_FIELDS
    reference = litellm_cache.CACHE_SETTINGS_FIELDS
    assert len(ours) == len(reference)
    assert [entry.field_name for entry in ours] == [entry.field_name for entry in reference]
    assert model_dumps(ours) == model_dumps(reference)


def test_cache_description_names() -> None:
    ours = betterlite_cache.REDIS_TYPE_DESCRIPTIONS
    reference = litellm_cache.REDIS_TYPE_DESCRIPTIONS
    assert ours == reference


def test_coordination_model_names() -> None:
    ours = betterlite_coordination.CoordinationRedisSettingsField
    reference = litellm_coordination.CoordinationRedisSettingsField
    assert list(ours.model_fields) == list(reference.model_fields)
    assert required_map(ours) == required_map(reference)
    assert default_map(ours) == default_map(reference)
    ours_hints: dict[str, JsonValue] = ours.__annotations__
    reference_hints: dict[str, JsonValue] = reference.__annotations__
    ours_section: tuple[JsonValue, ...] = get_args(ours_hints["section"])
    reference_section: tuple[JsonValue, ...] = get_args(reference_hints["section"])
    assert ours_section == reference_section


def test_literal_alias_names() -> None:
    ours_section_alias = betterlite_coordination.CoordinationRedisSection
    reference_section_alias = litellm_coordination.CoordinationRedisSection
    ours_section: tuple[JsonValue, ...] = get_args(ours_section_alias)
    reference_section: tuple[JsonValue, ...] = get_args(reference_section_alias)
    assert ours_section == reference_section
    ours_source_alias = betterlite_coordination.CoordinationRedisSource
    reference_source_alias = litellm_coordination.CoordinationRedisSource
    ours_source: tuple[JsonValue, ...] = get_args(ours_source_alias)
    reference_source: tuple[JsonValue, ...] = get_args(reference_source_alias)
    assert ours_source == reference_source


def test_coordination_field_names() -> None:
    ours = betterlite_coordination.COORDINATION_REDIS_SETTINGS_FIELDS
    reference = litellm_coordination.COORDINATION_REDIS_SETTINGS_FIELDS
    assert len(ours) == len(reference)
    assert [entry.field_name for entry in ours] == [entry.field_name for entry in reference]
    assert model_dumps(ours) == model_dumps(reference)


def test_router_create_names() -> None:
    ours = betterlite_router.FallbackCreateRequest
    reference = litellm_router.FallbackCreateRequest
    assert list(ours.model_fields) == list(reference.model_fields)
    assert required_map(ours) == required_map(reference)
    assert default_map(ours) == default_map(reference)
    ours_hints: dict[str, JsonValue] = ours.__annotations__
    reference_hints: dict[str, JsonValue] = reference.__annotations__
    ours_types: tuple[JsonValue, ...] = get_args(ours_hints["fallback_type"])
    reference_types: tuple[JsonValue, ...] = get_args(reference_hints["fallback_type"])
    assert ours_types == reference_types


def test_router_response_names() -> None:
    pairs: tuple[tuple[type[pydantic.BaseModel], type[pydantic.BaseModel]], ...] = (
        (betterlite_router.FallbackResponse, litellm_router.FallbackResponse),
        (betterlite_router.FallbackGetResponse, litellm_router.FallbackGetResponse),
        (betterlite_router.FallbackDeleteResponse, litellm_router.FallbackDeleteResponse),
    )
    for ours, reference in pairs:
        assert list(ours.model_fields) == list(reference.model_fields)
        assert required_map(ours) == required_map(reference)
        assert default_map(ours) == default_map(reference)


def test_router_field_names() -> None:
    ours_model = betterlite_router.RouterSettingsField
    reference_model = litellm_router.RouterSettingsField
    assert list(ours_model.model_fields) == list(reference_model.model_fields)
    assert required_map(ours_model) == required_map(reference_model)
    assert default_map(ours_model) == default_map(reference_model)
    ours = betterlite_router.ROUTER_SETTINGS_FIELDS
    reference = litellm_router.ROUTER_SETTINGS_FIELDS
    assert len(ours) == len(reference)
    assert [entry.field_name for entry in ours] == [entry.field_name for entry in reference]
    assert model_dumps(ours) == model_dumps(reference)


def test_router_description_names() -> None:
    ours = betterlite_router.ROUTING_STRATEGY_DESCRIPTIONS
    reference = litellm_router.ROUTING_STRATEGY_DESCRIPTIONS
    assert ours == reference


def test_fallback_validation_names() -> None:
    ours = betterlite_router.FallbackCreateRequest(model="gpt-4o", fallback_models=["gpt-4o-mini"])
    reference = litellm_router.FallbackCreateRequest(
        model="gpt-4o", fallback_models=["gpt-4o-mini"]
    )
    assert ours.fallback_type == "general"
    assert ours.model == "gpt-4o"
    ours_dump: dict[str, JsonValue] = ours.model_dump()
    reference_dump: dict[str, JsonValue] = reference.model_dump()
    assert ours_dump == reference_dump
    trimmed = betterlite_router.FallbackCreateRequest(model="  gpt-4o  ", fallback_models=["a"])
    assert trimmed.model == "gpt-4o"
    with pytest.raises(pydantic.ValidationError, match="must be a non-empty string"):
        _ = betterlite_router.FallbackCreateRequest(model="   ", fallback_models=["a"])
    with pytest.raises(pydantic.ValidationError, match="must not contain duplicates"):
        _ = betterlite_router.FallbackCreateRequest(model="m", fallback_models=["a", "a"])
    with pytest.raises(pydantic.ValidationError, match="at least 1 item"):
        _ = betterlite_router.FallbackCreateRequest(model="m", fallback_models=[])
    bad_type: dict[str, JsonValue] = {
        "model": "m",
        "fallback_models": ["a"],
        "fallback_type": "made-up",
    }
    with pytest.raises(pydantic.ValidationError, match="context_window"):
        _ = betterlite_router.FallbackCreateRequest.model_validate(bad_type)
