from typing import Final, Protocol, TypedDict, cast

from litellm.responses.litellm_completion_transformation.transformation import (
    LiteLLMCompletionResponsesConfig as Config,
)
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

WIRE = TypeAdapter(JsonValue)


class HiddenParams(TypedDict):
    _hidden_params: dict[str, JsonValue]


HIDDEN_PARAMS = TypeAdapter(HiddenParams)


class TransformResult(Protocol):
    __pydantic_private__: dict[str, JsonValue] | None

    def model_dump_json(self) -> str: ...


class TransformFn(Protocol):
    def __call__(
        self,
        request_input: str,
        responses_api_request: dict[str, JsonValue],
        chat_completion_response: ModelResponse,
    ) -> TransformResult: ...


CONFIG: Final = cast(TransformFn, Config.transform_chat_completion_response_to_responses_api_response)  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L2406


class HiddenParamsResponse(ModelResponse):
    def set_hidden(self, hidden: HiddenParams) -> None:
        self._hidden_params = hidden["_hidden_params"]


class ChatOutputInput(TypedDict):
    request_input: str
    responses_api_request: dict[str, JsonValue]
    chat_completion_response: dict[str, JsonValue]


def run(input_value: ChatOutputInput) -> JsonValue:
    chat = input_value["chat_completion_response"]
    response = HiddenParamsResponse.model_validate(chat)
    response.set_hidden(HIDDEN_PARAMS.validate_python(chat))
    result = CONFIG(
        request_input=input_value["request_input"],
        responses_api_request=input_value["responses_api_request"],
        chat_completion_response=response,
    )
    private = HIDDEN_PARAMS.validate_python(result.__pydantic_private__)
    return {
        "response": WIRE.validate_json(result.model_dump_json()),
        "hidden_params": private["_hidden_params"],
    }
