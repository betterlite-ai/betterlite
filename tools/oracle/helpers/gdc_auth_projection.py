import os
from typing import TypeAlias, cast
from unittest.mock import patch

import google.auth
import litellm
from litellm.llms.gdc.chat.transformation import GDCGeminiConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

FIXTURE_TOKEN: str = "fixture-token"  # noqa: S105 - a local fixture, never sent


class FixtureCredentials:
    def __init__(self, token: str = FIXTURE_TOKEN) -> None:
        self.token: str = token
        self.expiry: None = None

    @property
    def valid(self) -> bool:
        return True

    def with_gdch_audience(self, _audience: str) -> "FixtureCredentials":
        return FixtureCredentials(self.token)


def no_fetch_auth(_config: object, _creds: object, _ssl_verify: object) -> None:
    ...


def obj(value: Json) -> Object:
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise TypeError("expected an object")
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


def listing(fields: Object, name: str) -> list[Json]:
    value = fields.get(name)
    if value is None:
        return []
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return value


def apply_globals(config: Json) -> None:
    settings = obj(obj(config).get("litellm_settings"))
    litellm.api_base = optional_text(settings, "api_base")
    litellm.gdc_api_base = optional_text(settings, "gdc_api_base")
    litellm.vertex_project = optional_text(settings, "vertex_project")
    litellm.vertex_location = optional_text(settings, "vertex_location")


def deployment_params(config: Object, value: Json) -> Object:
    deployments = listing(config, "model_list")
    index = value if isinstance(value, int) and not isinstance(value, bool) else -1
    if index < 0 or index >= len(deployments):
        raise IndexError(f"deployment {value} is not configured")
    return obj(obj(deployments[index]).get("litellm_params"))


def header_pairs(fields: Object) -> dict[str, str]:
    headers: dict[str, str] = {}
    for entry in listing(fields, "headers"):
        pair = obj(entry)
        headers[text(pair, "name")] = text(pair, "value")
    return headers


def key_outcome(fields: Object) -> Json:
    instance = GDCGeminiConfig()
    fixture = FixtureCredentials()
    try:
        with patch.object(google.auth, "load_credentials_from_dict", return_value=(fixture, None)):
            _creds, is_service_account = instance._load_creds_from_key(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                text(fields, "api_key")
            )
    except Exception:  # noqa: BLE001 - every load failure is an authentication error
        return {"ok": False, "status": 401}
    kind = "service_account" if is_service_account else "bearer"
    return {"ok": True, "kind": kind}


def validation_outcome(fields: Object, config: Object, instance: GDCGeminiConfig) -> Json:
    params = deployment_params(config, fields.get("deployment"))
    fixture = FixtureCredentials()
    try:
        with (
            patch.object(google.auth, "load_credentials_from_dict", return_value=(fixture, None)),
            patch.object(GDCGeminiConfig, "_fetch_auth", no_fetch_auth),
        ):
            projected = cast(
                "dict[str, object]",
                instance.validate_environment(  # pyright: ignore[reportUnknownMemberType]
                    headers=header_pairs(fields),
                    model="gdc/gemini-2.5-pro",
                    messages=[],
                    optional_params=cast(
                        "dict[str, object]", obj(fields.get("optional_params"))
                    ),
                    litellm_params=cast("dict[str, object]", params),
                    api_key=text(fields, "api_key"),
                    api_base=optional_text(fields, "base"),
                ),
            )
    except Exception as error:  # noqa: BLE001 - the carried status is what a caller can branch on
        return {"ok": False, "status": cast(Json, getattr(error, "status_code", 500))}
    cache = [[audience, identity] for audience, identity in instance._gdch_creds_cache]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return {"ok": True, "headers": cast(Json, projected), "cache": cast(Json, cache)}


def scenario(fields: Object) -> Json:
    config = obj(fields.get("config"))
    apply_globals(config)
    for name in ("SSL_VERIFY", "GDC_TOKEN_CACHING"):
        os.environ.pop(name, None)
    environment = obj(fields.get("environment"))
    os.environ.update(
        {name: value for name, value in environment.items() if isinstance(value, str)}
    )
    instance = GDCGeminiConfig()
    return {
        "keys": [key_outcome(obj(entry)) for entry in listing(fields, "keys")],
        "validations": [
            validation_outcome(obj(entry), config, instance)
            for entry in listing(fields, "validations")
        ],
    }


def run(argument: Json) -> Json:
    fields = obj(argument)
    return {"scenarios": [scenario(obj(entry)) for entry in listing(fields, "scenarios")]}
