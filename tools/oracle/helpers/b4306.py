from typing import Literal, TypedDict

from litellm.proxy.management_endpoints.cache_settings_endpoints import (
    _has_connection_target,  # pyright: ignore[reportPrivateUsage]
    _is_credential_field,  # pyright: ignore[reportPrivateUsage]
    _merge_over_saved,  # pyright: ignore[reportPrivateUsage]
    _parse_stored_settings,  # pyright: ignore[reportPrivateUsage]
    _redact_credentials,  # pyright: ignore[reportPrivateUsage]
    _redact_settings,  # pyright: ignore[reportPrivateUsage]
    _resolve_cache_url_precedence,  # pyright: ignore[reportPrivateUsage]
    _saved_secret_is_reusable,  # pyright: ignore[reportPrivateUsage]
    _target_repr,  # pyright: ignore[reportPrivateUsage]
)


class UrlPrecedenceInput(TypedDict):
    function: Literal["url_precedence"]
    settings: dict[str, object]


class ParseStoredInput(TypedDict):
    function: Literal["parse_stored"]
    value: object


class RedactCredentialsInput(TypedDict):
    function: Literal["redact_credentials"]
    settings: dict[str, object]


class CredentialFieldInput(TypedDict):
    function: Literal["credential_field"]
    key: str


class HasTargetInput(TypedDict):
    function: Literal["has_target"]
    value: object


class TargetReprInput(TypedDict):
    function: Literal["target_repr"]
    value: object


class ReusableInput(TypedDict):
    function: Literal["reusable"]
    incoming: dict[str, object]
    saved: dict[str, object]


class MergeInput(TypedDict):
    function: Literal["merge"]
    incoming: dict[str, object]
    saved: dict[str, object]


class RedactSettingsInput(TypedDict):
    function: Literal["redact_settings"]
    settings: dict[str, object] | None


Input = (
    UrlPrecedenceInput
    | ParseStoredInput
    | RedactCredentialsInput
    | CredentialFieldInput
    | HasTargetInput
    | TargetReprInput
    | ReusableInput
    | MergeInput
    | RedactSettingsInput
)


def run(input_value: Input) -> object:
    match input_value["function"]:
        case "url_precedence":
            result = _resolve_cache_url_precedence(input_value["settings"])
        case "parse_stored":
            result = _parse_stored_settings(input_value["value"])
        case "redact_credentials":
            result = _redact_credentials(input_value["settings"])
        case "credential_field":
            result = _is_credential_field(input_value["key"])
        case "has_target":
            result = _has_connection_target(input_value["value"])
        case "target_repr":
            result = _target_repr(input_value["value"])
        case "reusable":
            result = _saved_secret_is_reusable(input_value["incoming"], input_value["saved"])
        case "merge":
            result = _merge_over_saved(input_value["incoming"], input_value["saved"])
        case "redact_settings":
            result = _redact_settings(input_value["settings"])
    return result
