import importlib
from collections.abc import Sequence
from typing import get_args

import pydantic
import pytest

import betterlite.types.management_endpoints.cache_settings_endpoints as betterlite_cache
import betterlite.types.management_endpoints.coordination_redis_endpoints as betterlite_coordination
import betterlite.types.management_endpoints.router_settings_endpoints as betterlite_router
from betterlite.types.utils import JsonValue

litellm_cache = importlib.import_module(
    "litellm.types.management_endpoints.cache_settings_endpoints"
)
litellm_coordination = importlib.import_module(
    "litellm.types.management_endpoints.coordination_redis_endpoints"
)
litellm_router = importlib.import_module(
    "litellm.types.management_endpoints.router_settings_endpoints"
)

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
    reference: Sequence[pydantic.BaseModel] = litellm_cache.CACHE_SETTINGS_FIELDS
    assert_entries_match(ours, reference)


def test_cache_description_names() -> None:
    ours = betterlite_cache.REDIS_TYPE_DESCRIPTIONS
    reference: dict[str, str] = litellm_cache.REDIS_TYPE_DESCRIPTIONS
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
    section: type[JsonValue] = litellm_coordination.CoordinationRedisSection
    reference_section: tuple[JsonValue, ...] = get_args(section)
    our_section: tuple[JsonValue, ...] = get_args(betterlite_coordination.CoordinationRedisSection)
    assert our_section == reference_section
    source: type[JsonValue] = litellm_coordination.CoordinationRedisSource
    reference_source: tuple[JsonValue, ...] = get_args(source)
    our_source: tuple[JsonValue, ...] = get_args(betterlite_coordination.CoordinationRedisSource)
    assert our_source == reference_source


def test_coordination_field_names() -> None:
    ours = betterlite_coordination.COORDINATION_REDIS_SETTINGS_FIELDS
    reference: Sequence[pydantic.BaseModel] = (
        litellm_coordination.COORDINATION_REDIS_SETTINGS_FIELDS
    )
    assert_entries_match(ours, reference)


def test_router_create_names() -> None:
    ours = model_names(CREATE_MODEL)
    reference = model_names(CREATE_REFERENCE)
    assert ours == reference
    hints: dict[str, JsonValue] = CREATE_MODEL.__annotations__
    reference_hints: dict[str, JsonValue] = CREATE_REFERENCE.__annotations__
    assert hints["fallback_type"] == reference_hints["fallback_type"]


def test_router_response_names() -> None:
    response: type[pydantic.BaseModel] = litellm_router.FallbackResponse
    get_response: type[pydantic.BaseModel] = litellm_router.FallbackGetResponse
    delete_response: type[pydantic.BaseModel] = litellm_router.FallbackDeleteResponse
    assert model_names(betterlite_router.FallbackResponse) == model_names(response)
    assert model_names(betterlite_router.FallbackGetResponse) == model_names(get_response)
    assert model_names(betterlite_router.FallbackDeleteResponse) == model_names(delete_response)


def test_router_field_names() -> None:
    ours = model_names(FIELD_MODEL)
    reference = model_names(FIELD_REFERENCE)
    assert ours == reference
    our_fields = betterlite_router.ROUTER_SETTINGS_FIELDS
    their_fields: Sequence[pydantic.BaseModel] = litellm_router.ROUTER_SETTINGS_FIELDS
    assert_entries_match(our_fields, their_fields)


def test_router_description_names() -> None:
    ours = betterlite_router.ROUTING_STRATEGY_DESCRIPTIONS
    reference: dict[str, str] = litellm_router.ROUTING_STRATEGY_DESCRIPTIONS
    assert ours == reference


def test_fallback_validation_names() -> None:
    ours = betterlite_router.FallbackCreateRequest(model="gpt-4o", fallback_models=["gpt-4o-mini"])
    reference: pydantic.BaseModel = litellm_router.FallbackCreateRequest(
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
