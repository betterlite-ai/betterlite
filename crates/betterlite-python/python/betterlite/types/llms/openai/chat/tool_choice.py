from typing import Literal, TypeAlias

from typing_extensions import Required, TypedDict


class ChatCompletionToolChoiceFunctionParam(TypedDict):  # ast-grep-ignore: no-long-names
    name: str


class ChatCompletionToolChoiceObjectParam(TypedDict):  # ast-grep-ignore: no-long-names
    type: Literal["function"]
    function: ChatCompletionToolChoiceFunctionParam  # ast-grep-ignore: no-long-names


ChatCompletionToolChoiceStringValues: TypeAlias = (  # ast-grep-ignore: no-long-names
    Literal["none", "auto", "required"]
)
ChatCompletionToolChoiceValues: TypeAlias = (
    # ast-grep-ignore: no-long-names
    ChatCompletionToolChoiceStringValues
    # ast-grep-ignore: no-long-names
    | ChatCompletionToolChoiceObjectParam
)


class Function(TypedDict, total=False):
    name: Required[str]


class ChatCompletionNamedToolChoiceParam(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    function: Required[Function]
    type: Required[Literal["function"]]
