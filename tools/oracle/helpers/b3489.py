from typing import Literal, TypedDict

from litellm.google_genai import main as g
from pydantic import JsonValue, TypeAdapter


class Input(TypedDict):
    entry: Literal["mock", "generate_content", "generate_content_stream"]
    model: str
    contents: JsonValue
    custom_llm_provider: str | None
    mock_response: str | None


INPUT: TypeAdapter[Input] = TypeAdapter(Input)
VALUE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


def run(input_value: JsonValue) -> JsonValue:
    call = INPUT.validate_python(input_value)
    entry = call["entry"]
    text = call["mock_response"]
    if entry == "mock":
        if text is None:
            return VALUE.validate_python(g.GenerateContentHelper.mock_generate_content_response())
        return VALUE.validate_python(
            g.GenerateContentHelper.mock_generate_content_response(mock_response=text)
        )
    if entry == "generate_content":
        return VALUE.validate_python(
            g.generate_content(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/google_genai/main.py#L302-L303
                model=call["model"],
                contents=call["contents"],
                custom_llm_provider=call["custom_llm_provider"],
                mock_response=text,
            )
        )
    return VALUE.validate_python(
        g.generate_content_stream(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/google_genai/main.py#L487-L488
            model=call["model"],
            contents=call["contents"],
            custom_llm_provider=call["custom_llm_provider"],
            mock_response=text,
        )
    )
