import litellm
import openai
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, ConfigDict, Field

MISSING = object()


class ExceptionFieldTypeError(TypeError):
    def __init__(self, name: str) -> None:
        super().__init__(f"exception field {name} is neither text nor null")


class StreamedResponseError(TypeError):
    def __init__(self) -> None:
        super().__init__(
            "litellm.completion returned a stream for a non-streaming call"
        )


class Message(BaseModel):
    model_config = ConfigDict(extra="forbid")

    role: str
    content: str


class ChatCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    messages: list[Message]
    api_key: str
    timeout_seconds: int


class Choice(BaseModel):
    content: str | None
    finish_reason: str | None


class ChatCompletion(BaseModel):
    id: str
    created: int
    object: str
    model: str | None
    choices: list[Choice]


class Success(BaseModel):
    value: ChatCompletion


class Failure(BaseModel):
    cls: str = Field(serialization_alias="class")
    status: int
    type: str | None = None
    param: str | None = None
    code: str | None = None
    message: str


class Outcome(BaseModel):
    success: Success | None = None
    failure: Failure | None = None


def optional_text(error: openai.APIStatusError, name: str) -> dict[str, str | None]:
    value: object = getattr(error, name, MISSING)
    if value is MISSING:
        return {}
    if value is None or isinstance(value, str):
        return {name: value}
    raise ExceptionFieldTypeError(name)


def project_failure(error: openai.APIStatusError) -> Outcome:
    fields: dict[str, str | int | None] = {
        "cls": type(error).__name__,
        "status": error.status_code,
        "message": error.message,
    }
    for name in ("type", "param", "code"):
        fields.update(optional_text(error, name))
    return Outcome(failure=Failure.model_validate(fields))


def project_success(response: ModelResponse) -> Outcome:
    choices = [
        Choice(content=choice.message.content, finish_reason=choice.finish_reason)
        for choice in response.choices
    ]
    value = ChatCompletion(
        id=response.id,
        created=response.created,
        object=response.object,
        model=response.model,
        choices=choices,
    )
    return Outcome(success=Success(value=value))


def run_chat(call: ChatCall, api_base: str) -> Outcome:
    try:
        response = litellm.completion(
            model=call.model,
            messages=[message.model_dump() for message in call.messages],
            api_key=call.api_key,
            api_base=api_base,
            timeout=call.timeout_seconds,
            custom_llm_provider="openai",
            num_retries=0,
        )
    except openai.APIStatusError as error:
        return project_failure(error)
    if not isinstance(response, ModelResponse):
        raise StreamedResponseError
    return project_success(response)
