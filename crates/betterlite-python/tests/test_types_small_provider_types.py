import importlib
import inspect
import ssl
from enum import Enum
from types import GenericAlias, UnionType
from typing import Final, Literal, TypeAlias

import pytest
from pydantic import BaseModel, create_model
from typing_extensions import (
    NotRequired,
    ReadOnly,
    Required,
    get_args,
    get_origin,
    get_type_hints,
    is_typeddict,
)

from betterlite.types.llms import base as betterlite_base
from betterlite.types.llms import databricks as betterlite_databricks
from betterlite.types.llms.base import (
    BaseLiteLLMOpenAIResponseObject,
    HiddenParams,
    LiteLLMPydanticObjectBase,
)
from betterlite.types.llms.custom_http import VerifyTypes
from betterlite.types.utils.base import InitValue

Hint: TypeAlias = type | GenericAlias | UnionType

MODULES: Final = (
    "aiml", "azure", "azure_ai", "base", "cohere", "custom_http", "databricks",
    "langgraph", "meta", "mistral", "ollama", "openrouter", "recraft", "rerank",
    "stability", "triton", "xai",
)
CONSTANTS: Final = {
    "azure": (
        "API_VERSION_YEAR_SUPPORTED_RESPONSE_FORMAT",
        "API_VERSION_MONTH_SUPPORTED_RESPONSE_FORMAT",
    ),
    "stability": (
        "OPENAI_SIZE_TO_STABILITY_ASPECT_RATIO",
        "STABILITY_ASPECT_RATIOS",
        "STABILITY_GENERATION_MODELS",
        "STABILITY_EDIT_ENDPOINTS",
    ),
}
ALIASES: Final = {
    "azure_ai": ("EncodingFormat",),
    "cohere": ("ChatHistory",),
    "databricks": ("AllDatabricksContentListValues", "AllDatabricksContentValues"),
    "meta": ("MuseMode", "MuseAudioEncoding", "MuseSampleRate"),
}
WRAPPERS: Final = (Required, NotRequired, ReadOnly)
MIRRORED_OPTIONS: Final = ("extra", "protected_namespaces")
RESPONSE_MS_KEY: Final = "_response_ms"


def signature(hint: Hint) -> set[str]:
    found: set[str] = set()
    stack = [hint]
    while stack:
        current = stack.pop()
        origin = get_origin(current)
        if origin is Literal:
            found.add(f"literal{sorted(repr(value) for value in get_args(current))}")
        elif origin is None:
            if is_typeddict(current):
                found.add(current.__name__)
        else:
            stack.extend(get_args(current))
    return found


def readonly_keys(typed_dict: type) -> set[str]:
    keys: set[str] = set()
    for key, annotation in get_type_hints(typed_dict, include_extras=True).items():
        current = annotation
        while get_origin(current) in WRAPPERS:
            if get_origin(current) is ReadOnly:
                keys.add(key)
            current = get_args(current)[0]
    return keys


def typed_dict_view(
    typed_dict: type, required: frozenset[str], optional: frozenset[str],
) -> dict[str, set[str] | frozenset[str]]:
    view: dict[str, set[str] | frozenset[str]] = {
        "required": required,
        "optional": optional,
        "readonly": readonly_keys(typed_dict),
    }
    for key, hint in get_type_hints(typed_dict).items():
        view[f"hint:{key}"] = signature(hint)
    return view


@pytest.mark.parametrize("module_name", MODULES)
def test_names_classes_match_litellm(module_name: str) -> None:
    litellm_module = importlib.import_module(f"litellm.types.llms.{module_name}")
    betterlite_module = importlib.import_module(f"betterlite.types.llms.{module_name}")
    ours = dict(inspect.getmembers(betterlite_module, inspect.isclass))
    for name, litellm_class in inspect.getmembers(litellm_module, inspect.isclass):
        if litellm_class.__module__ != litellm_module.__name__:
            continue
        assert name in ours, f"{module_name}.{name} is missing"
        betterlite_class = ours[name]
        if is_typeddict(litellm_class):
            assert is_typeddict(betterlite_class), name
            assert typed_dict_view(
                betterlite_class,
                betterlite_class.__required_keys__,
                betterlite_class.__optional_keys__,
            ) == typed_dict_view(
                litellm_class, litellm_class.__required_keys__, litellm_class.__optional_keys__,
            ), name
        elif Enum in litellm_class.__mro__:
            assert Enum in betterlite_class.__mro__, name
            assert str in betterlite_class.__mro__, name
            assert [(member.name, member.value) for member in betterlite_class] == [
                (member.name, member.value) for member in litellm_class
            ], name
        elif BaseModel in litellm_class.__mro__:
            assert BaseModel in betterlite_class.__mro__, name
            assert {
                field_name: (field.default, field.is_required())
                for field_name, field in betterlite_class.model_fields.items()
            } == {
                field_name: (field.default, field.is_required())
                for field_name, field in litellm_class.model_fields.items()
            }, name
            for option in MIRRORED_OPTIONS:
                assert betterlite_class.model_config.get(option) == litellm_class.model_config.get(
                    option,
                ), f"{name}.{option}"
        else:
            pytest.fail(f"{module_name}.{name} has a kind this test does not compare")


@pytest.mark.parametrize("module_name", sorted(CONSTANTS))
def test_names_constants_match_litellm(module_name: str) -> None:
    litellm_members = dict(
        inspect.getmembers(importlib.import_module(f"litellm.types.llms.{module_name}")),
    )
    betterlite_members = dict(
        inspect.getmembers(importlib.import_module(f"betterlite.types.llms.{module_name}")),
    )
    for name in CONSTANTS[module_name]:
        assert betterlite_members[name] == litellm_members[name], name


@pytest.mark.parametrize("module_name", sorted(ALIASES))
def test_names_aliases_match_litellm(module_name: str) -> None:
    litellm_members = dict(
        inspect.getmembers(importlib.import_module(f"litellm.types.llms.{module_name}")),
    )
    betterlite_members = dict(
        inspect.getmembers(importlib.import_module(f"betterlite.types.llms.{module_name}")),
    )
    for name in ALIASES[module_name]:
        assert signature(betterlite_members[name]) == signature(litellm_members[name]), name
        assert len(get_args(betterlite_members[name])) == len(get_args(litellm_members[name])), name


def test_names_verify_types_is_str_bool_ssl_context() -> None:
    assert set(get_args(VerifyTypes)) == {str, bool, ssl.SSLContext}


def test_names_databricks_usage_block_matches_litellm_openai_block() -> None:
    litellm_openai = importlib.import_module("litellm.types.llms.openai")
    ours = betterlite_databricks.ChatCompletionUsageBlock
    theirs = litellm_openai.ChatCompletionUsageBlock
    assert typed_dict_view(ours, ours.__required_keys__, ours.__optional_keys__) == typed_dict_view(
        theirs, theirs.__required_keys__, theirs.__optional_keys__,
    )


def test_names_triton_module_has_no_public_names() -> None:
    triton = importlib.import_module("betterlite.types.llms.triton")
    assert [name for name, _ in inspect.getmembers(triton) if not name.startswith("__")] == []


def test_names_hidden_params_defaults_match_litellm() -> None:
    litellm_base = importlib.import_module("litellm.types.llms.base")
    litellm_hidden = litellm_base.HiddenParams(model_id="deployment", extra_field=1)
    hidden = HiddenParams(model_id="deployment", extra_field=1)
    assert hidden.model_dump() == litellm_hidden.model_dump()
    assert hidden.json() == litellm_hidden.json()
    assert hidden.get("missing") == litellm_hidden.get("missing")
    assert hidden["extra_field"] == litellm_hidden["extra_field"]
    hidden["added"] = 3
    litellm_hidden["added"] = 3
    assert hidden.get("added") == litellm_hidden.get("added")
    assert hidden.model_dump()[RESPONSE_MS_KEY] is None
    with pytest.raises(AttributeError):
        _ = hidden["absent"]
    with pytest.raises(AttributeError):
        _ = litellm_hidden["absent"]


def test_names_pydantic_object_base_json_and_fields_set() -> None:
    litellm_base = importlib.import_module("litellm.types.llms.base")
    fields: dict[str, tuple[Hint, int | None]] = {"first": (int, 1), "second": (int | None, None)}
    ours = create_model("Sample", __base__=LiteLLMPydanticObjectBase, **fields)
    theirs = create_model("Sample", __base__=litellm_base.LiteLLMPydanticObjectBase, **fields)
    ours_sample = ours(first=3)
    theirs_sample = theirs(first=3)
    assert ours_sample.json() == theirs_sample.json()
    assert ours_sample.json(exclude_none=True) == theirs_sample.json(exclude_none=True)
    assert ours_sample.json(exclude_unset=True) == theirs_sample.json(exclude_unset=True)
    assert ours_sample.fields_set() == theirs_sample.fields_set()


def read_outcomes(sample: BaseLiteLLMOpenAIResponseObject, key: str) -> dict[str, InitValue]:
    try:
        item: InitValue = sample[key]
    except KeyError:
        item = "KeyError"
    return {
        "item": item,
        "get": sample.get(key),
        "contains": key in sample,
        "items": [[name, value] for name, value in sample.items()],
    }


@pytest.mark.parametrize("key", ["first", "second", "absent", "extra_field"])
def test_names_response_object_base_access_matches_litellm(key: str) -> None:
    litellm_base = importlib.import_module("litellm.types.llms.base")
    fields: dict[str, tuple[type, int]] = {"first": (int, 1), "second": (int, 2)}
    ours = create_model("Sample", __base__=BaseLiteLLMOpenAIResponseObject, **fields)
    theirs = create_model(
        "Sample", __base__=litellm_base.BaseLiteLLMOpenAIResponseObject, **fields,
    )
    assert read_outcomes(ours(first=5, extra_field=7), key) == read_outcomes(
        theirs(first=5, extra_field=7), key,
    )


def test_names_cached_tokens_details_defaults() -> None:
    litellm_base = importlib.import_module("litellm.types.llms.base")
    details = betterlite_base.CachedTokensDetails(text_tokens=2)
    assert details.model_dump() == litellm_base.CachedTokensDetails(text_tokens=2).model_dump()
    assert details.model_dump() == {"text_tokens": 2, "audio_tokens": None, "image_tokens": None}
