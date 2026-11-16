from collections.abc import Callable
from typing import Final, NotRequired, Protocol, TypedDict, TypeVar, runtime_checkable

from litellm.exceptions import BadRequestError, UnsupportedParamsError
from litellm.llms.azure_ai.image_generation import get_azure_ai_image_generation_config
from litellm.llms.azure_ai.image_generation.flux_transformation import (
    AzureFoundryFluxImageGenerationConfig as FluxConfig,
)
from pydantic import BaseModel, ConfigDict, JsonValue

Call = TypeVar("Call")
UNSUPPORTED: Final[type[Exception]] = UnsupportedParamsError
BAD_REQUEST: Final[type[Exception]] = BadRequestError


class UrlCall(TypedDict):
    model: str
    api_base: str | None
    api_version: str | None


class ParamsCall(TypedDict):
    model: str
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    drop_params: bool


class Input(TypedDict):
    models: NotRequired[list[str]]
    flux2: NotRequired[list[str]]
    paths: NotRequired[list[str]]
    urls: NotRequired[list[UrlCall]]
    supported: NotRequired[list[str]]
    params: NotRequired[list[ParamsCall]]


@runtime_checkable
class Config(Protocol):
    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def get_flux2_image_generation_url(
        self, api_base: str | None, model: str, api_version: str | None
    ) -> str: ...

    def is_flux2_model(self, model: str) -> bool: ...

    def get_flux2_provider_model_path(self, model: str) -> str: ...


class ConfigBoundary(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: Config


class FailureRecord(BaseModel):
    status_code: int | None = None
    llm_provider: str | None = None


CONFIG_VALUES: Final = {"config": FluxConfig()}
CONFIG: Config = ConfigBoundary.model_validate(CONFIG_VALUES).config


def outcomes(calls: list[Call], compute: Callable[[Call], JsonValue]) -> JsonValue:
    results: list[JsonValue] = []
    for call in calls:
        try:
            results.append({"value": compute(call)})
        except ValueError:
            results.append({"error": {"kind": "ValueError", "status": None, "provider": None}})
        except UNSUPPORTED as error:
            record = FailureRecord.model_validate(error, from_attributes=True)
            results.append(
                {
                    "error": {
                        "kind": "UnsupportedParamsError",
                        "status": record.status_code,
                        "provider": record.llm_provider,
                    }
                }
            )
        except BAD_REQUEST as error:
            record = FailureRecord.model_validate(error, from_attributes=True)
            results.append(
                {
                    "error": {
                        "kind": "BadRequestError",
                        "status": record.status_code,
                        "provider": record.llm_provider,
                    }
                }
            )
    return results


def supported_value(model: str) -> JsonValue:
    allowlist: list[JsonValue] = []
    allowlist.extend(CONFIG.get_supported_openai_params(model))
    return allowlist


def url_value(call: UrlCall) -> JsonValue:
    return CONFIG.get_flux2_image_generation_url(
        call["api_base"], call["model"], call["api_version"]
    )


def params_value(call: ParamsCall) -> JsonValue:
    return CONFIG.map_openai_params(
        call["non_default_params"],
        dict(call["optional_params"]),
        call["model"],
        drop_params=call["drop_params"],
    )


def run(values: Input) -> JsonValue:
    answer: dict[str, JsonValue] = {}
    if "models" in values:
        answer["models"] = [
            type(get_azure_ai_image_generation_config(model)).__name__
            for model in values["models"]
        ]
    if "flux2" in values:
        answer["flux2"] = [CONFIG.is_flux2_model(model) for model in values["flux2"]]
    if "paths" in values:
        answer["paths"] = [
            CONFIG.get_flux2_provider_model_path(model) for model in values["paths"]
        ]
    if "urls" in values:
        answer["urls"] = outcomes(values["urls"], url_value)
    if "supported" in values:
        answer["supported"] = [supported_value(model) for model in values["supported"]]
    if "params" in values:
        answer["params"] = outcomes(values["params"], params_value)
    return answer
