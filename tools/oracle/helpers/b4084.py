from typing import Final, Literal, Required, TypedDict

from litellm.integrations import shadow_eval_logger
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)

unmask_preference, tool_definitions_text, judge_user_prompt = (shadow_eval_logger._unmask_preference, shadow_eval_logger._tool_definitions_text, shadow_eval_logger._judge_user_prompt)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/shadow_eval_logger.py#L534-L578


class Input(TypedDict, total=False):
    operation: Required[
        Literal["tool_definitions", "judge_prompt", "unmask", "response_format", "system_prompt"]
    ]
    conversation: str
    response_a: str
    response_b: str
    tool_definitions: str
    tools: JsonValue
    raw_preference: str
    real_is_a: bool


def tool_definitions(input_value: Input) -> JsonValue:
    text = tool_definitions_text(input_value.get("tools"))
    return {"text": text}


def judge_prompt(input_value: Input) -> JsonValue:
    conversation = input_value.get("conversation")
    response_a = input_value.get("response_a")
    response_b = input_value.get("response_b")
    definitions = input_value.get("tool_definitions")
    text = judge_user_prompt(
        "" if conversation is None else conversation,
        "" if response_a is None else response_a,
        "" if response_b is None else response_b,
        "" if definitions is None else definitions,
    )
    return {"text": text}


def unmask(input_value: Input) -> JsonValue:
    raw = input_value.get("raw_preference")
    blind = input_value.get("real_is_a")
    preference = unmask_preference("" if raw is None else raw, blind is True)
    return {"preference": preference}


def response_format(_: Input) -> JsonValue:
    schema = JSON.validate_python(shadow_eval_logger.PAIRWISE_JUDGE_RESPONSE_FORMAT)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/shadow_eval_logger.py#L480
    return {"response_format": schema}


def run(input_value: Input) -> JsonValue:
    operation = input_value["operation"]
    if operation == "system_prompt":
        return {"text": shadow_eval_logger.PAIRWISE_JUDGE_SYSTEM_PROMPT}
    if operation == "tool_definitions":
        return tool_definitions(input_value)
    if operation == "judge_prompt":
        return judge_prompt(input_value)
    if operation == "unmask":
        return unmask(input_value)
    return response_format(input_value)
