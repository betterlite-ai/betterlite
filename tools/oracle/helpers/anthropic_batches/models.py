from pydantic import BaseModel, JsonValue, TypeAdapter

from .api import PreCall


class Bodies(BaseModel):
    now: int
    bodies: list[str]


class Create(BaseModel):
    environment: dict[str, str]
    url_bases: list[str | None]


class ErrorScenario(BaseModel):
    message: str
    status: int
    headers: tuple[tuple[str, str], ...] | None = None
    headers_object: tuple[tuple[str, str], ...] | None = None


class Errors(Bodies):
    errors: list[ErrorScenario]


class Scenario(BaseModel):
    environment: dict[str, str]
    batch_id: str
    api_base: str | None = None
    api_key: str | None = None
    sync: bool = False
    status: int = 200
    headers: tuple[tuple[str, str], ...] = ()
    body: str = ""


class Retrieve(BaseModel):
    now: int
    scenarios: list[Scenario]


class AuthScenario(BaseModel):
    environment: dict[str, str]
    api_key: str | None = None
    api_base: str | None = None
    params_api_base: str | None = None
    caller_headers: tuple[tuple[str, str], ...] = ()
    expected_auth: str | None = None


class Auth(BaseModel):
    scenarios: list[AuthScenario]


class PromptDetails(BaseModel):
    text_tokens: int
    cached_tokens: int
    cache_creation_tokens: int
    cache_write_tokens: int


class CompletionDetails(BaseModel):
    text_tokens: int
    reasoning_tokens: int
    accepted_prediction_tokens: int


class Usage(BaseModel):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: PromptDetails | None
    completion_tokens_details: CompletionDetails | None


class Content(BaseModel):
    content: str | None


class Choice(BaseModel):
    message: Content
    finish_reason: str | None


class Projection(BaseModel):
    id: str
    object: str
    created: int
    model: str | None
    choices: list[Choice]
    usage: Usage | None


class ErrorView(BaseModel):
    status_code: int
    message: str
    headers: dict[str, str] | None


REPLY: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PRE_CALL: TypeAdapter[PreCall] = TypeAdapter(PreCall)
PROJECTION: TypeAdapter[Projection] = TypeAdapter(Projection)
ZERO_PROMPT = PromptDetails(
    text_tokens=0, cached_tokens=0, cache_creation_tokens=0, cache_write_tokens=0
)
ZERO_COMPLETION = CompletionDetails(
    text_tokens=0, reasoning_tokens=0, accepted_prediction_tokens=0
)
