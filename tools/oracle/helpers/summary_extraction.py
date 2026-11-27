from typing import Final, TypedDict

from litellm.llms.anthropic.pass_through.context_management.editors.compact import (  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L1084
    _extract_response_text,
    _extract_usage,
)
from pydantic import BaseModel, JsonValue, TypeAdapter


class Message(BaseModel):
    content: JsonValue = None


class Choice(BaseModel):
    message: Message | None = None


class Usage(BaseModel):
    prompt_tokens: JsonValue = None
    completion_tokens: JsonValue = None


class Response(BaseModel):
    choices: list[Choice] | None = None
    usage: Usage | None = None


class Input(TypedDict):
    responses: list[JsonValue]


RESPONSE: Final[TypeAdapter[Response]] = TypeAdapter(Response)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(raw) for raw in input_value["responses"]]
    return {"answers": answers}


def answer(raw: JsonValue) -> dict[str, JsonValue]:
    response = RESPONSE.validate_python(raw)
    return {"text": text_answer(response), "usage": usage_answer(response)}


def text_answer(response: Response) -> JsonValue:
    try:
        text = _extract_response_text(response)
    except TypeError:
        return {"ok": None, "error": "TypeError"}
    return {"ok": text, "error": None}


def usage_answer(response: Response) -> JsonValue:
    try:
        input_tokens, output_tokens = _extract_usage(response)
    except ValueError:
        return {"ok": None, "error": "ValueError"}
    return {"ok": [input_tokens, output_tokens], "error": None}
