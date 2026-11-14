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

CACHE_MODEL: type[pydantic.BaseModel] = betterlite_cache.CacheSettingsField
CACHE_REFERENCE: type[pydantic.BaseModel] = litellm_cache.CacheSettingsField
COORDINATION_MODEL: type[pydantic.BaseModel] = (
    betterlite_coordination.CoordinationRedisSettingsField
)
COORDINATION_REFERENCE: type[pydantic.BaseModel] = (
    litellm_coordination.CoordinationRedisSettingsField
)
CREATE_MODEL: type[pydantic.BaseModel] = betterlite_router.FallbackCreateRequest
CREATE_REFERENCE: type[pydantic.BaseModel] = litellm_router.FallbackCreateRequest
FIELD_MODEL: type[pydantic.BaseModel] = betterlite_router.RouterSettingsField
FIELD_REFERENCE: type[pydantic.BaseModel] = litellm_router.RouterSettingsField


def model_names(
    model: type[pydantic.BaseModel],
) -> tuple[list[str], dict[str, bool], dict[str, JsonValue]]:
    fields = model.model_fields
    required: dict[str, bool] = {}
    defaults: dict[str, JsonValue] = {}
    for name, field in fields.items():
        required[name] = field.is_required()
        default: JsonValue = field.default
        defaults[name] = default
    return list(fields), required, defaults


def model_dumps(entries: Sequence[pydantic.BaseModel]) -> list[dict[str, JsonValue]]:
    dumped: list[dict[str, JsonValue]] = []
    for entry in entries:
        item: dict[str, JsonValue] = entry.model_dump()
        dumped.append(item)
    return dumped


def assert_entries_match(
    ours: Sequence[pydantic.BaseModel], reference: Sequence[pydantic.BaseModel]
) -> None:
    assert len(ours) == len(reference)
    assert model_dumps(ours) == model_dumps(reference)


def test_cache_model_names() -> None:
    ours = model_names(CACHE_MODEL)
    reference = model_names(CACHE_REFERENCE)
    assert ours == reference


def test_cache_field_names() -> None:
    ours = betterlite_cache.CACHE_SETTINGS_FIELDS
    reference = litellm_cache.CACHE_SETTINGS_FIELDS
    assert_entries_match(ours, reference)


def test_cache_description_names() -> None:
    ours = betterlite_cache.REDIS_TYPE_DESCRIPTIONS
    reference = litellm_cache.REDIS_TYPE_DESCRIPTIONS
    assert ours == reference


def test_json_narrowing_names() -> None:
    payload = {
        "field_name": "host",
        "field_type": "String",
        "field_value": object(),
        "field_description": "d",
        "ui_field_name": "Host",
    }
    with pytest.raises(pydantic.ValidationError, match="invalid-json-value"):
        _ = betterlite_cache.CacheSettingsField.model_validate(payload)


def test_coordination_model_names() -> None:
    ours = model_names(COORDINATION_MODEL)
    reference = model_names(COORDINATION_REFERENCE)
    assert ours == reference
    hints: dict[str, JsonValue] = COORDINATION_MODEL.__annotations__
    reference_hints: dict[str, JsonValue] = COORDINATION_REFERENCE.__annotations__
    assert hints["section"] == reference_hints["section"]


def test_literal_alias_names() -> None:
    section = betterlite_coordination.CoordinationRedisSection
    reference_section = litellm_coordination.CoordinationRedisSection
    section_values: tuple[JsonValue, ...] = get_args(section)
    reference_values: tuple[JsonValue, ...] = get_args(reference_section)
    assert section_values == reference_values
    source = betterlite_coordination.CoordinationRedisSource
    reference_source = litellm_coordination.CoordinationRedisSource
    source_values: tuple[JsonValue, ...] = get_args(source)
    reference_source_values: tuple[JsonValue, ...] = get_args(reference_source)
    assert source_values == reference_source_values


def test_coordination_field_names() -> None:
    ours = betterlite_coordination.COORDINATION_REDIS_SETTINGS_FIELDS
    reference = litellm_coordination.COORDINATION_REDIS_SETTINGS_FIELDS
    assert_entries_match(ours, reference)


def test_router_create_names() -> None:
    ours = model_names(CREATE_MODEL)
    reference = model_names(CREATE_REFERENCE)
    assert ours == reference
    hints: dict[str, JsonValue] = CREATE_MODEL.__annotations__
    reference_hints: dict[str, JsonValue] = CREATE_REFERENCE.__annotations__
    assert hints["fallback_type"] == reference_hints["fallback_type"]


def test_router_response_names() -> None:
    pairs: tuple[tuple[type[pydantic.BaseModel], type[pydantic.BaseModel]], ...] = (
        (betterlite_router.FallbackResponse, litellm_router.FallbackResponse),
        (betterlite_router.FallbackGetResponse, litellm_router.FallbackGetResponse),
        (betterlite_router.FallbackDeleteResponse, litellm_router.FallbackDeleteResponse),
    )
    for ours, reference in pairs:
        assert model_names(ours) == model_names(reference)


def test_router_field_names() -> None:
    ours = model_names(FIELD_MODEL)
    reference = model_names(FIELD_REFERENCE)
    assert ours == reference
    assert_entries_match(
        betterlite_router.ROUTER_SETTINGS_FIELDS, litellm_router.ROUTER_SETTINGS_FIELDS
    )


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
