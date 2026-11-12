from types import SimpleNamespace
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

from litellm.litellm_core_utils.llm_response_utils.get_formatted_prompt import (
    get_formatted_prompt,  # type: ignore[unused-ignore]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_response_utils/get_formatted_prompt.py#L5
)
from litellm.types.utils import (
    Choices,
    Delta,
    Message,
    ModelResponse,
    ModelResponseStream,
    StreamingChoices,
)
from litellm.utils import (
    _count_characters,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5383
    get_response_string,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class ContentPart(TypedDict, total=False):
    text: str


class OutputItem(TypedDict, total=False):
    content: list[ContentPart]
    text: str


class EventInput(TypedDict):
    event_type: str
    output: NotRequired[list[OutputItem]]
    delta: NotRequired[str]


class ChoiceInput(TypedDict, total=False):
    message: str | None
    delta: str | None


class ResponseInput(TypedDict, total=False):
    event: EventInput
    choices: list[ChoiceInput]


class CountCall(TypedDict):
    function: Literal["count_characters"]
    text: str


class PromptCall(TypedDict):
    function: Literal["formatted_prompt"]
    call_type: Literal[
        "acompletion",
        "completion",
        "text_completion",
        "embedding",
        "image_generation",
        "audio_transcription",
        "moderation",
    ]
    data: dict[str, Json]


class ResponseCall(TypedDict):
    function: Literal["response_string"]
    response: ResponseInput


Call: TypeAlias = CountCall | PromptCall | ResponseCall


class Input(TypedDict):
    calls: list[Call]


def output_object(item: OutputItem) -> SimpleNamespace:
    if "content" in item:
        parts: list[SimpleNamespace] = []
        for part in item["content"]:
            if "text" in part:
                parts.append(SimpleNamespace(text=part["text"]))
            else:
                parts.append(SimpleNamespace())
        return SimpleNamespace(content=parts)
    if "text" in item:
        return SimpleNamespace(text=item["text"])
    return SimpleNamespace()


def choice_object(choice: ChoiceInput) -> Choices | StreamingChoices:
    if "message" in choice:
        return Choices(message=Message(content=choice["message"]))
    return StreamingChoices(delta=Delta(content=choice.get("delta")))


def response_object(response: ResponseInput) -> SimpleNamespace:
    choices: list[Choices | StreamingChoices] = []
    if "choices" in response:
        choices = [choice_object(choice) for choice in response["choices"]]
    namespace = SimpleNamespace(choices=choices)
    if "event" not in response:
        return namespace
    event = response["event"]
    namespace.type = event["event_type"]
    output = event.get("output")
    if output is not None:
        items = [output_object(item) for item in output]
        namespace.response = SimpleNamespace(output=items)
    delta = event.get("delta")
    if delta is not None:
        namespace.delta = delta
    return namespace


def execute(call: Call) -> Json:
    match call["function"]:
        case "count_characters":
            return _count_characters(call["text"])
        case "formatted_prompt":
            return get_formatted_prompt(data=call["data"], call_type=call["call_type"])
        case "response_string":
            response = response_object(call["response"])
            typed = cast("ModelResponse | ModelResponseStream", response)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5389
            return get_response_string(response_obj=typed)


def run(input_value: Input) -> list[Json]:
    results: list[Json] = []
    for call in input_value["calls"]:
        try:
            results.append({"ok": True, "value": execute(call)})
        except (AttributeError, KeyError, TypeError, ValueError):
            results.append({"ok": False, "error": {"class": "ValueError"}})
    return results
