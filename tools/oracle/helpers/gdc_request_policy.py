import os
from types import SimpleNamespace
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.llms.gdc.chat.transformation import GDCGeminiConfig

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


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


def url_outcome(fields: Object, config: Object) -> Json:
    instance = GDCGeminiConfig()
    try:
        value = instance.get_complete_url(  # pyright: ignore[reportUnknownMemberType]
            api_base=optional_text(fields, "base"),
            api_key=None,
            model=text(fields, "model"),
            optional_params=cast("dict[str, object]", obj(fields.get("optional_params"))),
            litellm_params=cast(
                "dict[str, object]", deployment_params(config, fields.get("deployment"))
            ),
        )
    except Exception as error:  # noqa: BLE001 - the carried status is what a caller can branch on
        return {"ok": False, "status": cast(Json, getattr(error, "status_code", None))}
    return {"ok": True, "value": value}


def audience_outcome(fields: Object, config: Object) -> Json:
    instance = GDCGeminiConfig()
    credential = SimpleNamespace(valid=True, token="fixture-token")  # noqa: S106 - a local fixture, never sent
    params = deployment_params(config, fields.get("deployment"))
    try:
        with (
            patch.object(instance, "_load_creds_from_key", return_value=(credential, True)),
            patch.object(instance, "_fetch_auth"),
            patch.object(instance, "_with_gdch_audience", return_value=credential) as bound,
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
    except ValueError:
        # litellm.exception_type maps an unmapped ValueError to a 500 APIConnectionError
        return {"ok": False, "status": 500}
    return {"ok": True, "value": cast(Json, bound.call_args.args[1])}


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
    return {"ssl_verify": cast(Json, ssl_verify), "gdc_token_caching": cast(Json, caching),
            "caching_enabled": bool(caching)}


def supported_outcome(fields: Object) -> Json:
    instance = GDCGeminiConfig()
    chat_models = {name for name in listing(fields, "chat_models") if isinstance(name, str)}
    text_models = {name for name in listing(fields, "text_models") if isinstance(name, str)}
    with (
        patch.object(litellm, "open_ai_chat_completion_models", chat_models),
        patch.object(litellm, "open_ai_text_completion_models", text_models),
    ):
        supported = cast(
            "list[object]",
            instance.get_supported_openai_params(  # pyright: ignore[reportUnknownMemberType]
                text(fields, "model")
            ),
        )
    return cast(Json, [str(name) for name in supported])


def request_outcome(fields: Object) -> Json:
    instance = GDCGeminiConfig()
    messages = listing(fields, "messages")
    try:
        value = cast(
            "dict[str, object]",
            instance.transform_request(  # pyright: ignore[reportUnknownMemberType]
                model=text(fields, "model"),
                messages=cast("list[AllMessageValues]", messages),
                optional_params=cast(
                    "dict[str, object]", obj(fields.get("optional_params"))
                ),
                litellm_params={},
                headers={},
            ),
        )
    except Exception as error:  # noqa: BLE001 - the carried status is what a caller can branch on
        return {"ok": False, "status": cast(Json, getattr(error, "status_code", None))}
    return {"ok": True, "value": cast(Json, value)}


def scenario(fields: Object) -> Json:
    config = obj(fields.get("config"))
    apply_globals(config)
    for name in ("SSL_VERIFY", "GDC_TOKEN_CACHING"):
        os.environ.pop(name, None)
    environment = obj(fields.get("environment"))
    os.environ.update(
        {name: value for name, value in environment.items() if isinstance(value, str)}
    )
    return {
        "urls": [url_outcome(obj(entry), config) for entry in listing(fields, "urls")],
        "audiences": [
            audience_outcome(obj(entry), config) for entry in listing(fields, "audiences")
        ],
        "supported": [supported_outcome(obj(entry)) for entry in listing(fields, "supported")],
        "settings": [settings_outcome(obj(entry), config) for entry in listing(fields, "settings")],
        "requests": [request_outcome(obj(entry)) for entry in listing(fields, "requests")],
    }


def run(argument: Json) -> Json:
    fields = obj(argument)
    return {"scenarios": [scenario(obj(entry)) for entry in listing(fields, "scenarios")]}
