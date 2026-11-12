from collections.abc import Mapping
from typing import cast

import httpx
import litellm.llms.openai.chat.gpt_transformation as gpt_module
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.openai.chat.gpt_transformation import OpenAIGPTConfig
from litellm.llms.openai.common_utils import OpenAIError
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse

from .support import Json, Object, ResponseStep, as_object


class Logger:
    def __init__(self, events: list[Json]) -> None:
        self.events = events

    def post_call(self, **params: Json) -> None:
        additional = as_object(params.get("additional_args"))
        event: Object = {"name": "post_call"}
        event["input"] = params.get("input")
        event["original_response"] = params.get("original_response")
        event["complete_input_dict"] = additional.get("complete_input_dict")
        self.events.append(event)


class Recorder:
    def __init__(self, events: list[Json]) -> None:
        self.events = events

    def __call__(self, **kwargs: Json) -> ModelResponse:
        hidden = as_object(kwargs.get("hidden_params"))
        event: Object = {"name": "convert", "response": kwargs.get("response_object")}
        event["hidden_headers"] = hidden.get("headers")
        event["response_headers"] = kwargs.get("_response_headers")
        self.events.append(event)
        return ModelResponse()


def run(steps: list[ResponseStep]) -> Json:
    config = OpenAIGPTConfig()
    outputs: list[Json] = []
    for step in steps:
        events: list[Json] = []
        response = httpx.Response(
            step["status"], text=step["raw_text"], headers=step["raw_headers"]
        )
        del response.headers["content-length"]
        outcome: Object = {"kind": "reply"}
        records = Recorder(events)
        gpt_module.convert_to_model_response_object = records  # type: ignore[attr-defined, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L701-L706
        try:
            config.transform_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L661-L708
                model="gpt-4o",
                raw_response=response,
                model_response=ModelResponse(),
                logging_obj=cast(Logging, Logger(events)),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L676-L682
                request_data=step["request_data"],
                messages=cast(list[AllMessageValues], step["messages"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L665-L682
                optional_params={},  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L661-L708
                litellm_params={},  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L661-L708
                encoding=None,
                api_key=None,
                json_mode=False,
            )
        except OpenAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/common_utils.py#L57-L84
            headers = cast(Json, dict(cast(Mapping[str, str], error.headers)))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L687-L693
            outcome = {
                "kind": "error",
                "status_code": error.status_code,
                "headers": headers,
            }
        outputs.append({"events": events, "outcome": outcome})
    return outputs
