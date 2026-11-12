from typing import Literal, Protocol, TypeAlias, TypedDict

from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.text_to_speech.transformation import (
    BaseTextToSpeechConfig,
    TextToSpeechRequestData,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Headers: TypeAlias = dict[str, str]
EMPTY: Object = {}


class RecordInput(TypedDict, total=False):
    dict_body: Object
    ssml_body: str
    headers: Headers


class RequestContractInput(TypedDict):
    function: Literal["request_contract"]
    records: list[RecordInput]


class ClassConfigurationInput(TypedDict):
    function: Literal["class_configuration"]


class BaseUrlAndErrorInput(TypedDict):
    function: Literal["base_url_and_error"]
    headers: Headers
    api_bases: list[str | None]
    error_message: str
    status_code: int
    error_headers: Headers


Input: TypeAlias = RequestContractInput | ClassConfigurationInput | BaseUrlAndErrorInput


class MappingDefaults(TypedDict):
    voice: str | None
    drop_params: bool
    kwargs: Object


class RequestContractOutput(TypedDict):
    records: list[RecordInput]
    defaults: MappingDefaults


class ValueProjection(TypedDict):
    kind: Literal["value"]
    value: Json


class DescriptorProjection(TypedDict):
    kind: Literal["property", "callable"]


Projection: TypeAlias = ValueProjection | DescriptorProjection


class ClassConfigurationOutput(TypedDict):
    config: dict[str, Projection]


class UrlView(TypedDict):
    url: str | None
    error: str | None


class ErrorView(TypedDict):
    message: str
    status_code: int
    headers: Headers


class BaseUrlAndErrorOutput(TypedDict):
    validated: Headers
    urls: list[UrlView]
    error: ErrorView


class RecordFactory(Protocol):
    def __call__(
        self, *, dict_body: Object = ..., ssml_body: str = ..., headers: Headers = ...
    ) -> RecordInput: ...


RECORD: RecordFactory = TextToSpeechRequestData  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/text_to_speech/transformation.py#L25-L38


class CallableObject:
    def __call__(self) -> None: ...


class Calls(Protocol):
    def get_complete_url(self, model: str, api_base: str | None, litellm_params: Object) -> str: ...

    def validate_environment(self, headers: Headers, model: str) -> Headers: ...

    def get_error_class(self, error_message: str, status_code: int, headers: Headers) -> None: ...


class Subject(BaseTextToSpeechConfig): ...


# The base bodies run on an instance of a subclass that implements none of the abstract methods
Subject.__abstractmethods__ = frozenset()
SUBJECT: Calls = Subject()  # type: ignore[abstract,no-untyped-call,assignment]  # pyright: ignore[reportAbstractUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/text_to_speech/transformation.py#L80-L143


class Parent(Subject):
    voice = "alloy"


# Named `_` so the class body does not rewrite `__hidden` into `_Fixture__hidden`
class _(Parent):  # noqa: N801  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/text_to_speech/transformation.py#L42
    speed = 0
    enabled = False
    label = ""
    _private = 1
    __hidden = 2
    _abc_hidden = 3
    _abc = 4
    _ab = 5
    a__b = 6
    absent = None
    builtin = len
    callable_object = CallableObject()

    def method(self) -> None: ...

    @classmethod
    def class_method(cls) -> None: ...

    @staticmethod
    def static_method() -> None: ...

    @property
    def descriptor(self) -> None: ...


PROJECTIONS: dict[str, Projection] = {
    "speed": {"kind": "value", "value": 0},
    "enabled": {"kind": "value", "value": False},
    "label": {"kind": "value", "value": ""},
    "_private": {"kind": "value", "value": 1},
    "_ab": {"kind": "value", "value": 5},
    "a__b": {"kind": "value", "value": 6},
    "descriptor": {"kind": "property"},
    "callable_object": {"kind": "callable"},
}


def request_contract(input_value: RequestContractInput) -> RequestContractOutput:
    # LiteLLM declares voice=None, drop_params=False, kwargs={} on map_openai_params
    if BaseTextToSpeechConfig.map_openai_params.__defaults__ != (None, False, {}):  # type: ignore[misc] # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/text_to_speech/transformation.py#L67-L74
        raise ValueError("map_openai_params changed its declared defaults")
    return {
        "records": [RECORD(**record) for record in input_value["records"]],
        "defaults": {"voice": None, "drop_params": False, "kwargs": {}},
    }


def class_configuration() -> ClassConfigurationOutput:
    retained = _.get_config()  # type: ignore[no-untyped-call,misc] # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/text_to_speech/transformation.py#L42
    return {"config": {name: PROJECTIONS[name] for name in retained}}  # type: ignore[misc] # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/text_to_speech/transformation.py#L42


def base_url_and_error(input_value: BaseUrlAndErrorInput) -> BaseUrlAndErrorOutput:
    subject = SUBJECT
    urls: list[UrlView] = []
    for api_base in input_value["api_bases"]:
        try:
            urls.append({"url": subject.get_complete_url("m", api_base, EMPTY), "error": None})
        except ValueError as error:
            urls.append({"url": None, "error": str(error)})
    error_view: ErrorView = {"message": "", "status_code": 0, "headers": {}}
    try:
        subject.get_error_class(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/text_to_speech/transformation.py#L136-L143
            input_value["error_message"], input_value["status_code"], input_value["error_headers"]
        )
    except BaseLLMException as error:  # type: ignore[misc] # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41-L63
        error_view = {
            "message": error.message,
            "status_code": error.status_code,
            "headers": dict(error.headers),  # type: ignore[arg-type,misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41-L63
        }
    return {
        "validated": subject.validate_environment(input_value["headers"], "m"),
        "urls": urls,
        "error": error_view,
    }


def run(
    input_value: Input,
) -> RequestContractOutput | ClassConfigurationOutput | BaseUrlAndErrorOutput:
    match input_value["function"]:
        case "request_contract":
            return request_contract(input_value)
        case "class_configuration":
            return class_configuration()
        case "base_url_and_error":
            return base_url_and_error(input_value)
