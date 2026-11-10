import os
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.llms.gdc.chat.transformation import GDCGeminiConfig
from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Globals: TypeAlias = tuple[str | None, str | None, str | None, str | None]


class _AudienceCredential:
    def __init__(self) -> None:
        self.valid = True
        self.token = "fixture-token"  # noqa: S105 - a local fixture credential, never sent
        self.audience: str | None = None

    def with_gdch_audience(self, audience: str) -> "_AudienceCredential":
        self.audience = audience
        return self

    def __call__(self, _api_key: str) -> tuple["_AudienceCredential", bool]:
        return self, True


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def optional_obj(value: Json) -> Object:
    return {} if value is None else obj(value)


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


def strings(fields: Object, name: str) -> set[str]:
    return {value for value in listing(fields, name) if isinstance(value, str)}


def environment(fields: Object) -> dict[str, str]:
    raw = optional_obj(fields.get("environment"))
    variables = {name: value for name, value in raw.items() if isinstance(value, str)}
    if len(variables) != len(raw):
        raise TypeError("environment values must be strings")
    return variables


def apply_environment(fields: Object) -> None:
    variables = environment(fields)
    for name in ("SSL_VERIFY", "GDC_TOKEN_CACHING"):
        os.environ.pop(name, None)
    os.environ.update(variables)


def save_globals() -> Globals:
    return (
        litellm.api_base,
        litellm.gdc_api_base,
        litellm.vertex_project,
        litellm.vertex_location,
    )


def apply_globals(config: Object) -> None:
    settings = optional_obj(config.get("litellm_settings"))
    litellm.api_base = optional_text(settings, "api_base")
    litellm.gdc_api_base = optional_text(settings, "gdc_api_base")
    litellm.vertex_project = optional_text(settings, "vertex_project")
    litellm.vertex_location = optional_text(settings, "vertex_location")


def restore_globals(saved: Globals) -> None:
    (
        litellm.api_base,
        litellm.gdc_api_base,
        litellm.vertex_project,
        litellm.vertex_location,
    ) = saved


def deployment_params(config: Object, value: Json) -> Object:
    deployments = listing(config, "model_list")
    index = value if isinstance(value, int) and not isinstance(value, bool) else -1
    if index < 0 or index >= len(deployments):
        raise IndexError(f"deployment {value} is not configured")
    return optional_obj(obj(deployments[index]).get("litellm_params"))


def url_outcome(fields: Object, config: Object) -> Json:
    instance = GDCGeminiConfig()
    try:
        value = instance.get_complete_url(  # pyright: ignore[reportUnknownMemberType]
            api_base=optional_text(fields, "base"),
            api_key=None,
            model=text(fields, "model"),
            optional_params=cast("dict[str, object]", optional_obj(fields.get("optional_params"))),
            litellm_params=cast(
                "dict[str, object]", deployment_params(config, fields.get("deployment"))
            ),
        )
    except Exception as error:  # noqa: BLE001 - the carried status is what a caller can branch on
        return {"ok": False, "status": cast(Json, getattr(error, "status_code", None))}
    return {"ok": True, "value": value}


def audience_outcome(fields: Object, config: Object) -> Json:
    instance = GDCGeminiConfig()
    credential = _AudienceCredential()
    params = deployment_params(config, fields.get("deployment"))
    with (
        patch.object(instance, "_load_creds_from_key", credential),
        patch.object(instance, "_fetch_auth"),
    ):
        instance.validate_environment(  # pyright: ignore[reportUnknownMemberType]
            headers={},
            model="gdc/gemini-2.5-pro",
            messages=[],
            optional_params={},
            litellm_params=cast("dict[str, object]", params),
            api_key="fixture",
            api_base=text(fields, "base"),
        )
    return cast(Json, credential.audience)


def settings_outcome(fields: Object, config: Object) -> Json:
    instance = GDCGeminiConfig()
    params = deployment_params(config, fields.get("deployment"))
    ssl_verify = instance._read_env_bool(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        cast("bool | str | None", params.get("ssl_verify")), "SSL_VERIFY", default=True
    )
    caching = instance._read_env_bool(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        cast("bool | str | None", params.get("gdc_token_caching")),
        "GDC_TOKEN_CACHING",
        default=False,
    )
    return {
        "ssl_verify": cast(Json, ssl_verify),
        "gdc_token_caching": cast(Json, caching),
        "caching_enabled": bool(caching),
    }


def supported_outcome(fields: Object) -> Json:
    instance = GDCGeminiConfig()
    with (
        patch.object(litellm, "open_ai_chat_completion_models", strings(fields, "chat_models")),
        patch.object(litellm, "open_ai_text_completion_models", strings(fields, "text_models")),
    ):
        supported = cast(
            "list[object]",
            instance.get_supported_openai_params(  # pyright: ignore[reportUnknownMemberType]
                text(fields, "model")
            ),
        )
    return cast(Json, [str(name) for name in supported])


def messages(fields: Object) -> list[AllMessageValues]:
    value = fields.get("messages", [])
    if not isinstance(value, list):
        raise TypeError("messages must be a list")
    return cast("list[AllMessageValues]", value)


def request_outcome(fields: Object) -> Json:
    instance = GDCGeminiConfig()
    try:
        value = cast(
            "dict[str, object]",
            instance.transform_request(  # pyright: ignore[reportUnknownMemberType]
                model=text(fields, "model"),
                messages=messages(fields),
                optional_params=cast(
                    "dict[str, object]", optional_obj(fields.get("optional_params"))
                ),
                litellm_params={},
                headers={},
            ),
        )
    except Exception as error:  # noqa: BLE001 - the carried status is what a caller can branch on
        return {"ok": False, "status": cast(Json, getattr(error, "status_code", None))}
    return {"ok": True, "value": cast(Json, value)}


def scenario(fields: Object) -> Json:
    config = optional_obj(fields.get("config"))
    saved = save_globals()
    try:
        apply_globals(config)
        apply_environment(fields)
        return {
            "urls": [
                url_outcome(obj(entry), config) for entry in listing(fields, "urls")
            ],
            "audiences": [
                audience_outcome(obj(entry), config) for entry in listing(fields, "audiences")
            ],
            "supported": [
                supported_outcome(obj(entry)) for entry in listing(fields, "supported")
            ],
            "settings": [
                settings_outcome(obj(entry), config) for entry in listing(fields, "settings")
            ],
            "requests": [
                request_outcome(obj(entry)) for entry in listing(fields, "requests")
            ],
        }
    finally:
        restore_globals(saved)


def run(argument: Json) -> Json:
    fields = obj(argument)
    return {"scenarios": [scenario(obj(entry)) for entry in listing(fields, "scenarios")]}
