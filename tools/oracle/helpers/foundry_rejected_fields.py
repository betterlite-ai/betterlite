import io
from collections.abc import Callable
from contextlib import redirect_stdout
from typing import Final, NotRequired, TypeAlias, TypedDict

import httpx
import litellm
from litellm.llms.azure_ai.chat.transformation import AzureAIStudioConfig
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

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


# LiteLLM's three config members bind through their own names as aliases, typed here without Any
class RepairConfig(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    max_retry: int = Field(validation_alias="max_retry_on_unprocessable_entity_error")
    should_retry: Callable[[Exception, JsonObject], bool] = Field(
        validation_alias="should_retry_llm_api_inside_llm_translation_on_http_error"
    )
    transform_request: Callable[[Exception, JsonObject], JsonObject] = Field(
        validation_alias="transform_request_on_unprocessable_entity_error"
    )


CONFIG: Final[RepairConfig] = RepairConfig.model_validate(AzureAIStudioConfig())

litellm.suppress_debug_info = True


def rejected(error: str) -> httpx.HTTPStatusError:
    response = httpx.Response(422, text=error, request=REQUEST)
    return httpx.HTTPStatusError("rejected", request=REQUEST, response=response)


def trigger_value(call: TriggerCall) -> JsonValue:
    litellm.drop_params = call["global_drop_params"]
    params: JsonObject = {"drop_params": call["request_drop_params"]}
    retry = CONFIG.should_retry(rejected(call["error"]), params)
    return {"retry": retry}


def repair_value(call: RepairCall) -> JsonValue:
    repaired = CONFIG.transform_request(rejected(call["error"]), dict(call["body"]))
    return {"body": JSON.validate_python(repaired)}


def run(values: Input) -> JsonValue:
    answer: JsonObject = {}
    with redirect_stdout(io.StringIO()):
        if "triggers" in values:
            answer["triggers"] = [trigger_value(call) for call in values["triggers"]]
        if "repairs" in values:
            answer["repairs"] = [repair_value(call) for call in values["repairs"]]
        if values.get("bounds") is True:
            answer["bounds"] = {"max_retry": CONFIG.max_retry}
    return answer
