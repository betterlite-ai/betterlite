from typing import TypeAlias

from pydantic import JsonValue, PrivateAttr, TypeAdapter
from typing_extensions import override

from betterlite.types.llms.base import BaseLiteLLMOpenAIResponseObject
from betterlite.types.utils.base import InitValue

ContentListUnion: TypeAlias = JsonValue
ContentListUnionDict: TypeAlias = dict[str, JsonValue]
GenerateContentConfigOrDict: TypeAlias = dict[str, JsonValue]
# ast-grep-ignore: no-long-names
GoogleGenAIGenerateContentResponse: TypeAlias = dict[str, JsonValue]
# ast-grep-ignore: no-long-names
GenerateContentContentListUnionDict: TypeAlias = dict[str, JsonValue]


class GenerateContentConfigDict(dict[str, JsonValue]):
    @override
    def __init__(self, **kwargs: JsonValue) -> None:
        super().__init__(**kwargs)


# ast-grep-ignore: no-long-names
class GenerateContentRequestParametersDict(dict[str, JsonValue]):
    @override
    def __init__(self, **kwargs: JsonValue) -> None:
        super().__init__(**kwargs)


ToolConfigDict: TypeAlias = dict[str, JsonValue]


# ast-grep-ignore: no-long-names
class GenerateContentRequestDict(GenerateContentRequestParametersDict):
    @override
    def __init__(self, **kwargs: JsonValue) -> None:
        super().__init__(**kwargs)
        self.generationConfig = kwargs.get("generationConfig")
        self.tools = kwargs.get("tools")


# ast-grep-ignore: no-long-names
class GenerateContentResponse(BaseLiteLLMOpenAIResponseObject):
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    @override
    def __init__(self, **data: InitValue) -> None:
        hidden = data.pop("_hidden_params", None)
        super().__init__(**data)
        if hidden is not None:
            self._hidden_params = TypeAdapter(dict[str, JsonValue]).validate_python(hidden)
