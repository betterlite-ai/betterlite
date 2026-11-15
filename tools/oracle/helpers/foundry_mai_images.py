from collections.abc import Callable
from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, Protocol, TypedDict, TypeVar, runtime_checkable

import httpx
from litellm.exceptions import UnsupportedParamsError
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure_ai.image_generation.mai_transformation import (
    AzureFoundryMAIImageGenerationConfig as MaiConfig,
)
from litellm.llms.openai.common_utils import OpenAIError
from litellm.types.utils import ImageResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

Call = TypeVar("Call")
UNSUPPORTED: Final[type[Exception]] = UnsupportedParamsError
OPENAI_ERROR: Final[type[Exception]] = OpenAIError


class UrlCall(TypedDict):
    operation: Literal["generation", "edit"]
    api_base: str | None
    api_version: str | None


class ParamsCall(TypedDict):
    model: str
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    drop_params: bool


class ReplyCall(TypedDict):
    status_code: int
    body: str
    request_data: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    api_key: str | None


class Input(TypedDict):
    urls: NotRequired[list[UrlCall]]
    models: NotRequired[list[str]]
    params: NotRequired[list[ParamsCall]]
    usages: NotRequired[list[dict[str, JsonValue] | None]]
    replies: NotRequired[list[ReplyCall]]


@runtime_checkable
class Config(Protocol):
    def get_mai_image_generation_url(self, api_base: str | None, api_version: str | None) -> str: ...

    def get_mai_image_edit_url(self, api_base: str | None, api_version: str | None) -> str: ...

    def is_mai_model(self, model: str) -> bool: ...

    def normalize_mai_image_usage(
        self, usage: dict[str, JsonValue] | None
    ) -> dict[str, JsonValue]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def transform_image_generation_response(
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: ImageResponse,
        logging_obj: Logging,
        request_data: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        encoding: None,
        api_key: str | None = None,
    ) -> ImageResponse: ...


class ConfigBoundary(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: Config


class FailureRecord(BaseModel):
    status_code: int | None = None
    llm_provider: str | None = None


class LoggedDetails(BaseModel):
    input: str | None = None
    api_key: str | None = None
    original_response: str | None = None
    additional_args: dict[str, JsonValue] | None = None


class RecordedLogging(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: LoggedDetails


CONFIG_VALUES: Final = {"config": MaiConfig()}
CONFIG: Config = ConfigBoundary.model_validate(CONFIG_VALUES).config
OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


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
        except OPENAI_ERROR as error:
            record = FailureRecord.model_validate(error, from_attributes=True)
            results.append(
                {"error": {"kind": "OpenAIError", "status": record.status_code, "provider": None}}
            )
    return results


def url_value(call: UrlCall) -> JsonValue:
    if call["operation"] == "generation":
        return CONFIG.get_mai_image_generation_url(call["api_base"], call["api_version"])
    return CONFIG.get_mai_image_edit_url(call["api_base"], call["api_version"])


def params_value(call: ParamsCall) -> JsonValue:
    return CONFIG.map_openai_params(
        call["non_default_params"],
        dict(call["optional_params"]),
        call["model"],
        drop_params=call["drop_params"],
    )


def reply_value(call: ReplyCall) -> JsonValue:
    logging_obj = Logging(
        model="fixture-model",
        messages=None,
        stream=False,
        call_type="image_generation",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    response = CONFIG.transform_image_generation_response(
        model="MAI-Image-2.5",
        raw_response=httpx.Response(call["status_code"], content=call["body"].encode()),
        model_response=ImageResponse(),
        logging_obj=logging_obj,
        request_data=call["request_data"],
        optional_params=call["optional_params"],
        litellm_params={},
        encoding=None,
        api_key=call["api_key"],
    )
    details = RecordedLogging.model_validate(logging_obj).model_call_details
    logged: JsonValue = None
    if details.original_response is not None and details.additional_args is not None:
        logged = {
            "input": details.input,
            "api_key": details.api_key,
            "original_response": OBJECT.validate_json(details.original_response),
            "complete_input_dict": details.additional_args["complete_input_dict"],
        }
    return {"size": response.size, "logged": logged}


def run(values: Input) -> JsonValue:
    answer: dict[str, JsonValue] = {}
    if "urls" in values:
        answer["urls"] = outcomes(values["urls"], url_value)
    if "models" in values:
        answer["models"] = [CONFIG.is_mai_model(model) for model in values["models"]]
    if "params" in values:
        answer["params"] = outcomes(values["params"], params_value)
    if "usages" in values:
        answer["usages"] = outcomes(values["usages"], CONFIG.normalize_mai_image_usage)
    if "replies" in values:
        answer["replies"] = outcomes(values["replies"], reply_value)
    return answer
