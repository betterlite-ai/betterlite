import os
from collections.abc import Callable
from typing import TypeAlias, cast
from unittest import mock

import litellm
from litellm.llms.openai.common_utils import get_openai_credentials, is_openai_backed_api_base
from litellm.llms.openai.data_residency import infer_openai_data_residency
from litellm.main import _is_openai_backed_api_base, _resolve_openai_api_base  # pyright: ignore[reportPrivateUsage]

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected object")
    return value


def text(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def environment(fields: Object) -> dict[str, str]:
    raw = fields.get("env", {})
    if not isinstance(raw, dict):
        raise TypeError("env must be an object")
    variables = {name: value for name, value in raw.items() if isinstance(value, str)}
    if len(variables) != len(raw):
        raise TypeError("env values must be strings")
    return variables


def credentials(fields: Object) -> Json:
    with mock.patch.dict(os.environ, environment(fields), clear=True):
        litellm.api_base = None
        litellm.api_key = optional_text(fields, "litellm_api_key")
        litellm.openai_key = optional_text(fields, "litellm_openai_key")
        litellm.organization = optional_text(fields, "litellm_organization")
        resolved = get_openai_credentials(
            api_base=None,
            api_key=optional_text(fields, "api_key"),
            organization=optional_text(fields, "organization"),
        )
    return cast(
        Json,
        {
            "api_base": resolved.api_base,
            "api_key": resolved.api_key,
            "organization": resolved.organization,
        },
    )


def base_url(fields: Object) -> Json:
    api_base = optional_text(fields, "api_base")
    with mock.patch.dict(os.environ, environment(fields), clear=True):
        litellm.api_base = optional_text(fields, "litellm_api_base")
        resolved_main = _resolve_openai_api_base(api_base)
        resolved_common = get_openai_credentials(api_base=api_base).api_base
    if resolved_main != resolved_common:
        raise AssertionError(
            "main._resolve_openai_api_base and common_utils.get_openai_credentials diverged"
        )
    return {"api_base": resolved_main}


def host_boundary(fields: Object) -> Json:
    api_base = text(fields, "api_base")
    accepted_common = is_openai_backed_api_base(api_base)
    accepted_main = _is_openai_backed_api_base(api_base)
    if accepted_common != accepted_main:
        raise AssertionError(
            "common_utils.is_openai_backed_api_base and main._is_openai_backed_api_base diverged"
        )
    return {"accepted": accepted_common}


def residency(fields: Object) -> Json:
    return infer_openai_data_residency(
        optional_text(fields, "custom_llm_provider"), optional_text(fields, "api_base")
    )


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "key_and_organization_precedence": credentials,
    "base_url_before_api_base": base_url,
    "empty_organization": credentials,
    "openai_host_boundary": host_boundary,
    "regional_data_residency": residency,
}


def run(argument: Json) -> Json:
    fields = obj(argument)
    if len(fields) != 1:
        raise ValueError("expected one openai_credentials variant")
    name, value = next(iter(fields.items()))
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown openai_credentials case: {name}")
    return handler(obj(value))
