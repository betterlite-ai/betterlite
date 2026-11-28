from pydantic import JsonValue
from typing_extensions import Required, TypedDict


class OpenAIChatCompletionLogprobsContentTopLogprobs(  # ast-grep-ignore: no-long-names
    TypedDict,
    total=False,
):
    bytes: list[JsonValue]
    logprob: Required[float]
    token: Required[str]


class OpenAIChatCompletionLogprobsContent(  # ast-grep-ignore: no-long-names
    TypedDict,
    total=False,
):
    bytes: list[JsonValue]
    logprob: Required[float]
    token: Required[str]
    top_logprobs: list[
        OpenAIChatCompletionLogprobsContentTopLogprobs  # ast-grep-ignore: no-long-names
    ]
