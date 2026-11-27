import os
from dataclasses import dataclass, field
from typing import Final, Literal, Protocol, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.image_edit.handler import BedrockImageEdit
from litellm.llms.bedrock.image_generation.image_handler import BedrockImageGeneration
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.router_utils.add_retry_fallback_headers import get_hidden_params_dict
from litellm.types.utils import ImageResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

COST_HEADER: Final = "llm_provider-x-litellm-response-cost"
NOW: Final = 1_700_000_000
GENERATION_CASES: Final = ("titan_image", "stability_image", "canvas_image")


class Input(TypedDict):
    case: Literal[
        "titan_image",
        "stability_image",
        "canvas_image",
        "canvas_edit",
        "stability_edit",
    ]
    model_cost: dict[str, JsonValue]
    probes: list[dict[str, JsonValue]]


@dataclass
class Reply:
    status: int = 200
    headers: dict[str, str] = field(default_factory=dict)
    body: str = '{"images": ["eA=="]}'


@dataclass
class Probe:
    model: str
    prompt: str | None = None
    images: list[str] = field(default_factory=list)
    optional_params: dict[str, JsonValue] = field(default_factory=dict)
    extra_headers: dict[str, str] | None = None
    reply: Reply = field(default_factory=Reply)


PROBE: Final[TypeAdapter[Probe]] = TypeAdapter(Probe)
BODY: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class Hidden(TypedDict):
    additional_headers: dict[str, JsonValue]


HIDDEN: Final[TypeAdapter[Hidden]] = TypeAdapter(Hidden)


@runtime_checkable
class Logger(Protocol):
    def pre_call(self, **values: JsonValue) -> None: ...

    def post_call(self, **values: JsonValue) -> None: ...


class Recorder:
    def __init__(self) -> None:
        self.pre: dict[str, JsonValue] | None = None
        self.post: dict[str, JsonValue] | None = None

    def pre_call(self, **values: JsonValue) -> None:
        self.pre = values

    def post_call(self, **values: JsonValue) -> None:
        self.post = values


@runtime_checkable
class GenerationHandler(Protocol):
    def image_generation(
        self,
        model: str,
        prompt: str,
        model_response: ImageResponse,
        optional_params: dict[str, JsonValue],
        logging_obj: Logger,
        timeout: float | None,
        *,
        aimg_generation: bool = False,
        api_base: str | None = None,
        extra_headers: dict[str, str] | None = None,
        client: HTTPHandler | None = None,
        api_key: str | None = None,
    ) -> ImageResponse: ...


@runtime_checkable
class EditHandler(Protocol):
    def image_edit(
        self,
        model: str,
        image: list[str],
        prompt: str | None,
        model_response: ImageResponse,
        optional_params: dict[str, JsonValue],
        logging_obj: Logger,
        timeout: float | None,
        *,
        aimage_edit: bool = False,
        api_base: str | None = None,
        extra_headers: dict[str, str] | None = None,
        client: HTTPHandler | None = None,
        api_key: str | None = None,
    ) -> ImageResponse: ...


class GenerationBinding(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    handler: GenerationHandler


class EditBinding(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    handler: EditHandler


def generation_handler() -> GenerationHandler:
    binding = GenerationBinding.model_validate({"handler": BedrockImageGeneration()})
    return binding.handler


def edit_handler() -> EditHandler:
    binding = EditBinding.model_validate({"handler": BedrockImageEdit()})
    return binding.handler


def prepare_environment() -> None:
    os.environ.clear()
    os.environ["AWS_CONFIG_FILE"] = os.devnull
    os.environ["AWS_SHARED_CREDENTIALS_FILE"] = os.devnull


class Fixture:
    def __init__(self, reply: Reply, extra_name: str | None) -> None:
        self.reply = reply
        self.extra_name = extra_name
        self.requests: list[httpx.Request] = []
        prepare_environment()

    def client(self) -> HTTPHandler:
        transport = httpx.MockTransport(self.respond)
        return HTTPHandler(client=httpx.Client(transport=transport))

    def respond(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        return httpx.Response(
            self.reply.status,
            headers=self.reply.headers,
            content=self.reply.body.encode(),
        )


def first_header_name(headers: dict[str, str] | None) -> str | None:
    if headers is None:
        return None
    return next(iter(sorted(headers)), None)


def authorization(request: httpx.Request) -> JsonValue:
    value = request.headers.get("authorization")
    if value is None:
        return None
    if value.startswith("AWS4-HMAC-SHA256"):
        return "sigv4"
    return "other"


def extra_header(request: httpx.Request, name: str | None) -> JsonValue:
    if name is None:
        return None
    return request.headers.get(name)


def request_view(fixture: Fixture) -> dict[str, JsonValue]:
    if not fixture.requests:
        return {
            "url": None,
            "body": None,
            "authorization": None,
            "extra_header": None,
            "posted": 0,
        }
    request = fixture.requests[0]
    return {
        "url": str(request.url),
        "body": BODY.validate_json(request.content),
        "authorization": authorization(request),
        "extra_header": extra_header(request, fixture.extra_name),
        "posted": len(fixture.requests),
    }


def images_of(response: ImageResponse) -> list[JsonValue]:
    data = response.data
    if data is None:
        return []
    return [image.b64_json for image in data if image.b64_json is not None]


def edit_cost(response: ImageResponse) -> JsonValue:
    hidden = HIDDEN.validate_python(get_hidden_params_dict(response))
    return hidden["additional_headers"].get(COST_HEADER)


def success_row(
    fixture: Fixture, logger: Recorder, images: list[JsonValue], cost: JsonValue
) -> JsonValue:
    return {
        "request": request_view(fixture),
        "ok": True,
        "status": None,
        "images": images,
        "cost": cost,
        "post_call": logger.post is not None,
    }


def failure_row(fixture: Fixture, logger: Recorder, status: int | None) -> JsonValue:
    empty: list[JsonValue] = []
    return {
        "request": request_view(fixture),
        "ok": False,
        "status": status,
        "images": empty,
        "cost": None,
        "post_call": logger.post is not None,
    }


def generation_prompt(probe: Probe) -> str:
    prompt = probe.prompt
    if prompt is None:
        raise ValueError("a generation probe carries a prompt")
    return prompt


def generation_row(probe: Probe) -> JsonValue:
    fixture = Fixture(probe.reply, first_header_name(probe.extra_headers))
    logger = Recorder()
    try:
        response = generation_handler().image_generation(
            probe.model,
            generation_prompt(probe),
            ImageResponse(created=NOW),
            dict(probe.optional_params),
            logger,
            None,
            aimg_generation=False,
            api_base=None,
            extra_headers=None,
            client=fixture.client(),
            api_key=None,
        )
    except BedrockError as error:
        return failure_row(fixture, logger, error.status_code)
    return success_row(fixture, logger, images_of(response), None)


def edit_row(probe: Probe) -> JsonValue:
    fixture = Fixture(probe.reply, first_header_name(probe.extra_headers))
    logger = Recorder()
    try:
        response = edit_handler().image_edit(
            probe.model,
            probe.images,
            probe.prompt,
            ImageResponse(),
            dict(probe.optional_params),
            logger,
            None,
            aimage_edit=False,
            api_base=None,
            extra_headers=probe.extra_headers,
            client=fixture.client(),
            api_key=None,
        )
    except BedrockError as error:
        return failure_row(fixture, logger, error.status_code)
    except ValueError:
        return failure_row(fixture, logger, None)
    return success_row(fixture, logger, images_of(response), edit_cost(response))


def run(input_value: Input) -> JsonValue:
    litellm.model_cost = input_value["model_cost"]
    edit = input_value["case"] not in GENERATION_CASES
    rows: list[JsonValue] = []
    for probe in input_value["probes"]:
        parsed = PROBE.validate_python(probe)
        if edit:
            rows.append(edit_row(parsed))
        else:
            rows.append(generation_row(parsed))
    return rows
