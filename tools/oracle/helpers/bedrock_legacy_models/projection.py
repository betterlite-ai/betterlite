from typing import TypedDict

from litellm.llms.bedrock.chat.invoke_transformations.amazon_ai21_transformation import (
    AmazonAI21Config,
)
from litellm.llms.bedrock.chat.invoke_transformations.anthropic_claude2_transformation import (
    AmazonAnthropicConfig,
)
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

from .constructors import FamilyName, LegacyClass, project


def _ignored(_cls: type[LegacyClass]) -> None: ...


# A fresh class dict carries _abc_impl; Claude2's limit is the import-time litellm.max_tokens.
class Ai21Kinds(AmazonAI21Config):
    def _method(self) -> None: ...

    __marked__ = "dunder"
    plain = 5
    text = "value"
    nothing = None
    _private = "kept"
    _abc_value = "abc"
    function = _method
    builtin = len
    class_method = classmethod(_ignored)
    static_method = staticmethod(_ignored)
    descriptor = property()


class AnthropicKinds(AmazonAnthropicConfig):
    def _method(self) -> None: ...

    __marked__ = "dunder"
    plain = 5
    text = "value"
    nothing = None
    _private = "kept"
    _abc_value = "abc"
    function = _method
    builtin = len
    class_method = classmethod(_ignored)
    static_method = staticmethod(_ignored)
    descriptor = property()


class ProjectionCase(TypedDict):
    family: FamilyName
    attributes: dict[str, Json]


def fixture(family: FamilyName) -> LegacyClass:
    if family == "anthropic":
        return AnthropicKinds()
    if family == "ai21":
        return Ai21Kinds()
    raise ValueError(f"no fixture class for {family}")


def projection(raw: Json) -> Json:
    case = TypeAdapter(ProjectionCase).validate_python(raw)
    return {"family": case["family"], "config": project(fixture(case["family"]))}
