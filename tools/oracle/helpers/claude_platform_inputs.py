import os

# LiteLLM reads LITELLM_RUST per call; false pins the Python fallback the oracle compares against.
os.environ["LITELLM_RUST"] = "false"

from base64 import b64encode
from collections.abc import Callable, Generator, Mapping, Sequence
from contextlib import contextmanager
from datetime import UTC, datetime
from types import SimpleNamespace
from typing import TypeAlias, cast
from unittest.mock import patch

import botocore.auth
import litellm
from botocore.credentials import Credentials
from litellm.exceptions import AuthenticationError
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.claude_platform import common_utils
from litellm.llms.bedrock.claude_platform.common_utils import (
    BedrockClaudePlatformMixin,
    filter_claude_platform_request_body,
    resolve_unsupported_override,
    strip_claude_platform_route,
)
from pydantic import BaseModel, Field, JsonValue

Json: TypeAlias = JsonValue
Params: TypeAlias = dict[str, Json]
Scalar: TypeAlias = str | int | float | bool | None

OVERRIDE_KEY = "claude_platform_unsupported_params"


class WorkspaceCase(BaseModel):
    optional_params: Params = Field(default_factory=dict)
    deployment: dict[str, str] = Field(default_factory=dict)
    litellm_settings: dict[str, str] = Field(default_factory=dict)
    environment: dict[str, str] = Field(default_factory=dict)


class RegionCase(BaseModel):
    optional_params: Params = Field(default_factory=dict)
    environment: dict[str, str] = Field(default_factory=dict)


class UrlCase(BaseModel):
    api_base: str | None = None
    litellm_api_base: str | None = None
    optional_params: Params = Field(default_factory=dict)
    environment: dict[str, str] = Field(default_factory=dict)


class OverrideCall(BaseModel):
    deployment: Json = None
    optional: Json = None
    log_invalid: bool = True


class BodyCall(BaseModel):
    params: Params = Field(default_factory=dict)
    unsupported: list[str] | None = None
    log_dropped: bool = True


class Keys(BaseModel):
    access_key_id: str
    secret_access_key: str
    session_token: str | None = None

    def to_boto(self) -> Credentials:
        return Credentials(
            access_key=self.access_key_id,
            secret_key=self.secret_access_key,
            token=self.session_token,
        )


class Sign(BaseModel):
    headers: dict[str, str | None] = Field(default_factory=dict)
    optional_params: Params = Field(default_factory=dict)
    request_data: Params = Field(default_factory=dict)
    api_base: str
    api_key: str | None = None
    model: str | None = None
    environment: dict[str, str] = Field(default_factory=dict)
    credentials: Keys
    now: int


class ErrorClass(BaseModel):
    message: str
    status_code: int
    headers: list[tuple[str, str]] = Field(default_factory=list)


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def scalar_items(values: Sequence[Json]) -> list[Scalar]:
    items: list[Scalar] = []
    for value in values:
        if isinstance(value, (str, int, float, bool)) or value is None:
            items.append(value)
        else:
            raise ValueError("set items must be scalars")
    return items


def override_container(value: Json) -> object:
    fields = object_value(value, "unsupported params")
    if len(fields) != 1:
        raise ValueError("an unsupported params override names exactly one container")
    kind, payload = next(iter(fields.items()))
    values = list_value(payload, "override items")
    if kind == "list":
        return values
    if kind == "set":
        return set(scalar_items(values))
    if kind == "frozenset":
        return frozenset(scalar_items(values))
    if kind == "tuple":
        return tuple(values)
    if kind == "other":
        return payload
    raise ValueError(f"unknown override container: {kind}")


@contextmanager
def captured_warnings() -> Generator[list[Json], None, None]:
    entries: list[Json] = []

    def record(message: str, *args: object) -> None:
        if message.startswith(
            "bedrock/claude_platform: dropping unsupported Messages API param(s) %s"
        ):
            entries.append(
                {"action": "drop", "params": list(cast("Sequence[str]", args[0]))}
            )
        elif message.startswith(
            "bedrock/claude_platform: ignoring claude_platform_unsupported_params of type %s"
        ):
            entries.append({"action": "ignore", "type_name": str(args[0])})
        else:
            raise ValueError(f"unexpected warning: {message}")

    with patch.object(
        common_utils.verbose_logger,  # pyright: ignore[reportPrivateImportUsage]
        "warning",
        record,
    ):
        yield entries


def workspace_alias_precedence(payload: Json) -> Json:
    cases = list_value(object_value(payload, "payload").get("cases"), "cases")
    workspaces: list[Json] = []
    for value in cases:
        case = WorkspaceCase.model_validate(value)
        litellm_params: Params = {**case.litellm_settings, **case.deployment}
        with patch.dict(os.environ, case.environment, clear=True):
            workspaces.append(
                BedrockClaudePlatformMixin._get_workspace_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
                    dict(case.optional_params), litellm_params
                )
            )
    return {"workspaces": workspaces}


def explicit_region_required(payload: Json) -> Json:
    cases = list_value(object_value(payload, "payload").get("cases"), "cases")
    regions: list[Json] = []
    for value in cases:
        case = RegionCase.model_validate(value)
        with patch.dict(os.environ, case.environment, clear=True):
            try:
                region = BedrockClaudePlatformMixin()._get_required_aws_region_name(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
                    dict(case.optional_params)
                )
            except AuthenticationError:
                regions.append({"error": "missing_region"})
            except ValueError:
                regions.append({"error": "invalid_region"})
            except Exception:  # noqa: BLE001 - a step reports every fixture failure
                regions.append({"error": "other"})
            else:
                regions.append({"region": region})
    return {"regions": regions}


def api_base_before_region(payload: Json) -> Json:
    cases = list_value(object_value(payload, "payload").get("cases"), "cases")
    urls: list[Json] = []
    for value in cases:
        case = UrlCase.model_validate(value)
        with (
            patch.dict(os.environ, case.environment, clear=True),
            patch.object(litellm, "api_base", case.litellm_api_base),
        ):
            try:
                url = BedrockClaudePlatformMixin().get_complete_url(  # pyright: ignore[reportUnknownMemberType]
                    case.api_base,
                    None,
                    "",
                    dict(case.optional_params),
                    {},
                )
            except AuthenticationError:
                urls.append({"error": "missing_region"})
            except ValueError:
                urls.append({"error": "invalid_region"})
            except Exception:  # noqa: BLE001 - a step reports every fixture failure
                urls.append({"error": "other"})
            else:
                urls.append({"url": url})
    return {"urls": urls}


def platform_route_prefix(payload: Json) -> Json:
    models = list_value(object_value(payload, "payload").get("models"), "models")
    return {
        "models": [
            strip_claude_platform_route(model)
            for model in models
            if isinstance(model, str)
        ]
    }


def unsupported_override(payload: Json) -> Json:
    calls = list_value(object_value(payload, "payload").get("calls"), "calls")
    outcomes: list[Json] = []
    for value in calls:
        call = OverrideCall.model_validate(value)
        litellm_params: dict[str, object] = {}
        if call.deployment is not None:
            litellm_params[OVERRIDE_KEY] = override_container(call.deployment)
        optional_params: dict[str, object] = {}
        if call.optional is not None:
            optional_params[OVERRIDE_KEY] = override_container(call.optional)
        with captured_warnings() as entries:
            resolved = resolve_unsupported_override(
                litellm_params,
                optional_params=optional_params,
                log_invalid=call.log_invalid,
            )
        names: Json = cast(Json, sorted(resolved) if resolved is not None else None)
        warning: Json = entries[0] if entries else None
        outcomes.append({"names": names, "warning": warning})
    return {"calls": outcomes}


def filtered_body_fields(payload: Json) -> Json:
    calls = list_value(object_value(payload, "payload").get("calls"), "calls")
    outcomes: list[Json] = []
    for value in calls:
        call = BodyCall.model_validate(value)
        unsupported = (
            frozenset(call.unsupported) if call.unsupported is not None else None
        )
        with captured_warnings() as entries:
            body = filter_claude_platform_request_body(
                dict(call.params),
                unsupported_override=unsupported,
                log_dropped=call.log_dropped,
            )
        outcomes.append({"body": cast(Json, body), "warnings": entries})
    return {"calls": outcomes}


@contextmanager
def signing_environment(
    environment: Mapping[str, str],
    credentials: Credentials,
    now: int,
) -> Generator[None, None, None]:
    frozen = datetime.fromtimestamp(now, tz=UTC).replace(tzinfo=None)

    def resolve(_self: object, _auth: object, _region: object) -> Credentials:
        return credentials

    with (
        patch.dict(os.environ, dict(environment), clear=True),
        patch("boto3.Session", lambda: SimpleNamespace(region_name=None)),
        patch.object(botocore.auth, "get_current_datetime", lambda: frozen),
        patch.object(BaseAWSLLM, "resolve_credentials", resolve),
    ):
        yield


def sign(value: Json) -> Json:
    case = Sign.model_validate(value)
    with signing_environment(case.environment, case.credentials.to_boto(), case.now):
        signature = cast(
            "tuple[dict[str, Json], bytes | None]",
            BedrockClaudePlatformMixin().sign_request(  # pyright: ignore[reportUnknownMemberType]
                headers=dict(case.headers),
                optional_params=dict(case.optional_params),
                request_data=dict(case.request_data),
                api_base=case.api_base,
                api_key=case.api_key,
                model=case.model,
            ),
        )
    headers = signature[0]
    body = signature[1]
    if body is None:
        return {"signed": False, "headers": headers, "body": None}
    return {
        "signed": True,
        "method": "POST",
        "url": case.api_base,
        "headers": headers,
        "body": b64encode(body).decode(),
    }


def error_class(value: Json) -> Json:
    case = ErrorClass.model_validate(value)
    error = BedrockClaudePlatformMixin().get_error_class(
        case.message, case.status_code, dict(case.headers)
    )
    return {"ok": True, "status_code": error.status_code}


def platform_signing(payload: Json) -> Json:
    calls = list_value(object_value(payload, "payload").get("calls"), "calls")
    outcomes: list[Json] = []
    for value in calls:
        fields = object_value(value, "signing call")
        if len(fields) != 1:
            raise ValueError("a signing call names exactly one kind")
        kind, body = next(iter(fields.items()))
        if kind == "sign":
            outcomes.append(sign(body))
        elif kind == "error_class":
            outcomes.append(error_class(body))
        else:
            raise ValueError(f"unknown signing call: {kind}")
    return {"calls": outcomes}


HANDLERS: dict[str, Callable[[Json], Json]] = {
    "workspace_alias_precedence": workspace_alias_precedence,
    "explicit_region_required": explicit_region_required,
    "api_base_before_region": api_base_before_region,
    "platform_route_prefix": platform_route_prefix,
    "unsupported_override": unsupported_override,
    "filtered_body_fields": filtered_body_fields,
    "platform_signing": platform_signing,
}


def run(input_value: dict[str, Json]) -> Json:
    _ = litellm
    if len(input_value) != 1:
        raise ValueError("the input names exactly one case variant")
    variant, payload = next(iter(input_value.items()))
    handler = HANDLERS.get(variant)
    if handler is None:
        raise ValueError(f"unknown variant: {variant}")
    return handler(payload)
