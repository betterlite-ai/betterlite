import io
from contextlib import redirect_stdout
from typing import Final, NotRequired, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.llms.azure_ai.chat.transformation import AzureAIStudioConfig
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
REQUEST: Final[httpx.Request] = httpx.Request("POST", "https://oracle.test/v1/chat/completions")
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class TriggerCall(TypedDict):
    error: str
    request_drop_params: bool
    global_drop_params: bool


class RepairCall(TypedDict):
    error: str
    body: JsonObject


class Input(TypedDict):
    triggers: NotRequired[list[TriggerCall]]
    repairs: NotRequired[list[RepairCall]]
    bounds: NotRequired[bool]


# the config's own signatures carry Any, so its three members enter through this JSON-facing shape
@runtime_checkable
class RepairConfig(Protocol):
    @property
    def max_retry_on_unprocessable_entity_error(self) -> int: ...  # ast-grep-ignore: no-long-names

    def should_retry_llm_api_inside_llm_translation_on_http_error(  # ast-grep-ignore: no-long-names
        self, e: Exception, litellm_params: JsonObject
    ) -> bool: ...

    def transform_request_on_unprocessable_entity_error(  # ast-grep-ignore: no-long-names
        self, e: Exception, request_data: JsonObject
    ) -> JsonObject: ...


class ConfigBoundary(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: RepairConfig


CONFIG_VALUES: Final = {"config": AzureAIStudioConfig()}
CONFIG: Final[RepairConfig] = ConfigBoundary.model_validate(CONFIG_VALUES).config

litellm.suppress_debug_info = True


def rejected(error: str) -> httpx.HTTPStatusError:
    response = httpx.Response(422, text=error, request=REQUEST)
    return httpx.HTTPStatusError("rejected", request=REQUEST, response=response)


def trigger_value(call: TriggerCall) -> JsonValue:
    litellm.drop_params = call["global_drop_params"]
    params: JsonObject = {"drop_params": call["request_drop_params"]}
    retry = CONFIG.should_retry_llm_api_inside_llm_translation_on_http_error(
        rejected(call["error"]), params
    )
    return {"retry": retry}


def repair_value(call: RepairCall) -> JsonValue:
    repaired = CONFIG.transform_request_on_unprocessable_entity_error(
        rejected(call["error"]), dict(call["body"])
    )
    return {"body": JSON.validate_python(repaired)}


def run(values: Input) -> JsonValue:
    answer: JsonObject = {}
    with redirect_stdout(io.StringIO()):
        if "triggers" in values:
            answer["triggers"] = [trigger_value(call) for call in values["triggers"]]
        if "repairs" in values:
            answer["repairs"] = [repair_value(call) for call in values["repairs"]]
        if values.get("bounds") is True:
            answer["bounds"] = {"max_retry": CONFIG.max_retry_on_unprocessable_entity_error}
    return answer
