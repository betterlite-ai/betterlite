from collections.abc import Callable, Mapping
from typing import Protocol, TypeAlias, cast

import httpx
import litellm
from litellm.llms.azure.chat.gpt_transformation import (
    AzureOpenAIConfig,
    sanitized_tools_update,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


class ErrorView(Protocol):
    status_code: int
    status_code_is_synthesized: bool
    message: str
    headers: dict[str, str] | httpx.Headers | None
    request: httpx.Request
    response: httpx.Response
    body: dict[str, object] | None


class AzureConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def requires_max_completion_tokens(self, model: str) -> bool: ...

    def _is_response_format_supported_model(self, model: str) -> bool: ...

    def _is_response_format_supported_api_version(
        self, api_version_year: str, api_version_month: str
    ) -> bool: ...

    def map_openai_params(
        self,
        non_default_params: Object,
        optional_params: Object,
        model: str,
        *,
        drop_params: bool,
        api_version: str = "",
    ) -> Object: ...

    def transform_request(
        self,
        model: str,
        messages: list[Json],
        optional_params: Object,
        litellm_params: Object,
        headers: Object,
    ) -> Json: ...

    def get_mapped_special_auth_params(self) -> dict[str, str]: ...

    def map_special_auth_params(
        self, non_default_params: Object, optional_params: Object
    ) -> Object: ...

    def get_eu_regions(self) -> list[str]: ...

    def get_us_regions(self) -> list[str]: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, str] | None
    ) -> ErrorView: ...

    def get_config(self) -> Object: ...


class ConfigFactory(Protocol):
    def __call__(self, **values: Json) -> AzureConfig: ...


def as_object(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return cast("Object", value)


def as_list(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return cast("list[Json]", value)


def as_string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def as_bool(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise TypeError(f"{name} must be a boolean")
    return value


def as_int(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(f"{name} must be an integer")
    return value


def status_of(error: Exception) -> int | None:
    carried: object = getattr(error, "status_code", None)
    return carried if isinstance(carried, int) and not isinstance(carried, bool) else None


def config_for() -> AzureConfig:
    return cast(AzureConfig, AzureOpenAIConfig())


def apply_settings(call: dict[str, Json]) -> tuple[bool, bool]:
    settings = as_object(call.get("settings", {}), "settings")
    drop_params = as_bool(settings.get("drop_params", False), "drop_params")
    litellm.drop_params = drop_params
    system_first = as_bool(
        settings.get("openai_system_messages_first", False),
        "openai_system_messages_first",
    )
    litellm.openai_system_messages_first = system_first
    return drop_params, system_first


def strings(value: Json) -> list[str]:
    return [as_string(item, "model") for item in as_list(value, "values")]


def map_step(config: AzureConfig, raw: Json, *, global_drop: bool) -> Json:
    step = as_object(raw, "step")
    override = step.get("global_drop_params")
    litellm.drop_params = (
        global_drop if override is None else as_bool(override, "global_drop_params")
    )
    optional = dict(as_object(step.get("optional", {}), "optional"))
    try:
        mapped = config.map_openai_params(
            as_object(step.get("non_default", {}), "non_default"),
            optional,
            as_string(step.get("model"), "model"),
            drop_params=as_bool(step.get("drop_params", False), "drop_params"),
            api_version=as_string(step.get("api_version", ""), "api_version"),
        )
    except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
        return {"ok": False, "value": None, "status": status_of(error)}
    return {"ok": True, "value": cast(Json, mapped), "status": None}


def mapping_steps(config: AzureConfig, call: dict[str, Json], *, global_drop: bool) -> Json:
    return [
        map_step(config, step, global_drop=global_drop)
        for step in as_list(call.get("steps", []), "steps")
    ]


def model_report(config: AzureConfig, model: str) -> Json:
    supported = config._is_response_format_supported_model(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        model
    )
    return {"model": model, "supported": supported}


def version_report(config: AzureConfig, raw: Json) -> Json:
    pair = as_object(raw, "version")
    year = as_string(pair.get("year"), "year")
    month = as_string(pair.get("month"), "month")
    try:
        supported = config._is_response_format_supported_api_version(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            year, month
        )
    except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
        return {"ok": False, "supported": None, "status": status_of(error)}
    return {"ok": True, "supported": supported, "status": None}


def version_reports(call: dict[str, Json]) -> Json:
    config = config_for()
    drop_params, _system_first = apply_settings(call)
    models = [model_report(config, model) for model in strings(call.get("models", []))]
    versions = [
        version_report(config, pair)
        for pair in as_list(call.get("versions", []), "versions")
    ]
    return cast(
        Json,
        {
            "models": models,
            "versions": versions,
            "steps": mapping_steps(config, call, global_drop=drop_params),
        },
    )


def predicate_report(call: dict[str, Json]) -> Json:
    config = config_for()
    drop_params, _system_first = apply_settings(call)
    flags = [
        {"model": model, "requires": config.requires_max_completion_tokens(model)}
        for model in strings(call.get("models", []))
    ]
    return cast(
        Json,
        {
            "models": flags,
            "steps": mapping_steps(config, call, global_drop=drop_params),
        },
    )


def supported_report(call: dict[str, Json]) -> Json:
    config = config_for()
    drop_params, _system_first = apply_settings(call)
    lists = [
        {"model": model, "params": config.get_supported_openai_params(model)}
        for model in strings(call.get("models", []))
    ]
    return cast(
        Json,
        {
            "models": lists,
            "steps": mapping_steps(config, call, global_drop=drop_params),
        },
    )


def transform_step(config: AzureConfig, raw: Json, *, system_first: bool) -> Json:
    step = as_object(raw, "step")
    override = step.get("openai_system_messages_first")
    litellm.openai_system_messages_first = (
        system_first if override is None else as_bool(override, "openai_system_messages_first")
    )
    try:
        body = config.transform_request(
            model="gpt-4o",
            messages=as_list(step.get("messages", []), "messages"),
            optional_params=as_object(step.get("optional", {}), "optional"),
            litellm_params={},
            headers={},
        )
    except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
        return {"ok": False, "value": None, "status": status_of(error)}
    return {"ok": True, "value": body, "status": None}


def transform_steps(call: dict[str, Json]) -> Json:
    config = config_for()
    _drop_params, system_first = apply_settings(call)
    return [
        transform_step(config, step, system_first=system_first)
        for step in as_list(call.get("steps", []), "steps")
    ]


def transform_report(call: dict[str, Json]) -> Json:
    return {"steps": transform_steps(call)}


def mapping_report(call: dict[str, Json]) -> Json:
    config = config_for()
    drop_params, _system_first = apply_settings(call)
    return {"steps": mapping_steps(config, call, global_drop=drop_params)}


def sanitize_report(call: dict[str, Json]) -> Json:
    optional = as_object(call.get("optional", {}), "optional")
    update: Mapping[str, object] = sanitized_tools_update(cast("Mapping[str, object]", optional))
    return cast(Json, dict(update))


def class_reports(call: dict[str, Json]) -> Json:
    factory = cast(ConfigFactory, AzureOpenAIConfig)
    reports: list[Json] = []
    for raw in as_list(call.get("steps", []), "steps"):
        values = as_object(as_object(raw, "step").get("values", {}), "values")
        reports.append(cast(Json, factory(**values).get_config()))
    return {"steps": reports}


def auth_report(call: dict[str, Json]) -> Json:
    config = config_for()
    steps: list[Json] = []
    for raw in as_list(call.get("steps", []), "steps"):
        step = as_object(raw, "step")
        optional = dict(as_object(step.get("optional", {}), "optional"))
        config.map_special_auth_params(
            as_object(step.get("non_default", {}), "non_default"), optional
        )
        steps.append(cast(Json, optional))
    return {
        "mapped": cast(Json, config.get_mapped_special_auth_params()),
        "eu": cast(Json, config.get_eu_regions()),
        "us": cast(Json, config.get_us_regions()),
        "steps": steps,
    }


def error_report(config: AzureConfig, raw: Json) -> Json:
    step = as_object(raw, "step")
    raw_headers = step.get("headers")
    headers = (
        None
        if raw_headers is None
        else cast("dict[str, str]", as_object(raw_headers, "headers"))
    )
    error = config.get_error_class(
        as_string(step.get("message"), "message"),
        as_int(step.get("status_code"), "status_code"),
        headers,
    )
    response = error.response
    return cast(
        Json,
        {
            "status_code": error.status_code,
            "message": error.message,
            "headers": None
            if error.headers is None
            else [list(pair) for pair in error.headers.items()],
            "request": {"method": error.request.method, "url": str(error.request.url)},
            "response": {
                "status_code": response.status_code,
                "request": {
                    "method": response.request.method,
                    "url": str(response.request.url),
                },
            },
            "body": error.body,
            "status_code_is_synthesized": error.status_code_is_synthesized,
        },
    )


def error_reports(call: dict[str, Json]) -> Json:
    config = config_for()
    return {
        "steps": [
            error_report(config, step)
            for step in as_list(call.get("steps", []), "steps")
        ]
    }


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "api_version_response_format": version_reports,
    "max_completion_tokens": predicate_report,
    "unsupported_param": supported_report,
    "azure_tool_choice_date_gate": mapping_report,
    "azure_required_tool_choice": mapping_report,
    "azure_parameter_shapes": mapping_report,
    "azure_message_content": transform_report,
    "sanitized_tool_schema": sanitize_report,
    "azure_tool_choice_without_tools": transform_report,
    "azure_class_defaults": class_reports,
    "azure_special_auth_regions": auth_report,
    "azure_error_factory": error_reports,
}


def run(input_value: dict[str, Json]) -> Json:
    action = as_string(input_value.get("action"), "action")
    handler = HANDLERS.get(action)
    if handler is None:
        raise ValueError(f"unsupported action {action}")
    return handler(input_value)
