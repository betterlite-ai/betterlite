import os
from typing import Final, Protocol, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.azure import AzureChatCompletion
from litellm.llms.azure.common_utils import AzureOpenAIError
from litellm.llms.azure.image_edit.transformation import AzureImageEditConfig
from litellm.llms.azure.image_generation import get_azure_image_generation_config
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


class Step(TypedDict, total=False):
    azure_endpoint: str
    base_url: str
    api_base: str
    api_version: str
    model: str
    base_model: str
    data: dict[str, JsonValue]
    request_url: str
    global_base: str
    global_version: str
    headers: dict[str, str]
    api_key: str
    params: dict[str, JsonValue]
    env_token: str
    prompt: str
    optional: dict[str, JsonValue]
    azure_ad_token: str
    reply_body: str


class Input(TypedDict, total=False):
    targets: list[Step]
    models: list[str]
    forms: list[Step]
    urls: list[Step]
    auth: list[Step]
    generations: list[Step]


class ImageDatum(TypedDict):
    b64_json: str | None
    url: str | None


class ImageReply(TypedDict):
    created: int | None
    data: list[ImageDatum]


# the typed view of AzureImageEditConfig, whose own methods take bare dicts
class EditConfig(Protocol):
    # ast-grep-ignore: no-long-names
    def azure_deployment_image_edit_form_data(
        self, data: dict[str, JsonValue], request_url: str
    ) -> dict[str, JsonValue]: ...

    def validate_environment(
        self,
        *,
        headers: dict[str, str],
        model: str,
        api_key: str | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(
        self, *, model: str, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...


# the typed view of AzureChatCompletion.image_generation, whose own signature has bare dicts
class CompletionApi(Protocol):
    def create_azure_base_url(
        self,
        *,
        azure_client_params: dict[str, JsonValue],
        model: str | None,
        base_model: str | None = None,
    ) -> str: ...

    def image_generation(  # noqa: PLR0913  # LiteLLM's own parameter list: https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L1228-L1249
        self,
        *,
        prompt: str,
        timeout: float,
        optional_params: dict[str, JsonValue],
        logging_obj: Logging,
        headers: dict[str, str],
        model: str | None = None,
        api_key: str | None = None,
        api_base: str | None = None,
        api_version: str | None = None,
        azure_ad_token: str | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
    ) -> ImageResponse: ...


JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
REPLY: Final[TypeAdapter[ImageReply]] = TypeAdapter(ImageReply)
EDIT_CONFIG: EditConfig = cast("EditConfig", AzureImageEditConfig())  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_edit/transformation.py#L29-L30
COMPLETION: CompletionApi = cast("CompletionApi", AzureChatCompletion())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L1228-L1360
FAILURE: Final[dict[str, JsonValue]] = {"kind": "error", "type": None, "param": None, "code": None}


def clear_ambient() -> None:
    for name in [key for key in os.environ if key.startswith("AZURE_")]:
        os.environ.pop(name, None)
    litellm.api_key = litellm.azure_key = litellm.api_base = litellm.api_version = None


class Transport(httpx.MockTransport):
    def __init__(self, reply: str) -> None:
        super().__init__(self.respond)
        self.reply = reply
        self.calls: list[dict[str, JsonValue]] = []

    def respond(self, request: httpx.Request) -> httpx.Response:
        content = request.content
        body: JsonValue = None if not content else JSON.validate_json(content)
        headers: dict[str, str] = dict(request.headers)
        seen: dict[str, JsonValue] = {
            name: headers.get(name) for name in ("api-key", "authorization")
        }
        self.calls.append({"sent_url": str(request.url), "sent_body": body, "sent_headers": seen})
        return httpx.Response(status_code=200, content=self.reply.encode(), request=request)


class RecordingLogging(Logging):
    def __init__(self) -> None:
        self.pre_inputs: list[str] = []
        self.posts: list[str] = []

    @override
    def pre_call(self, *args: JsonValue, **values: JsonValue) -> None:
        record = args[0] if args else values["input"]
        self.pre_inputs.append(str(record))

    @override
    def post_call(self, *args: JsonValue, **values: JsonValue) -> None:
        record = args[0] if args else values["input"]
        self.posts.append(str(record))


def model_step(model: str) -> JsonValue:
    config = get_azure_image_generation_config(model)
    return JSON.validate_python({"class": config.__class__.__name__})


def target_step(step: Step) -> JsonValue:
    params: dict[str, JsonValue] = {}
    for name, value in (
        ("azure_endpoint", step.get("azure_endpoint")),
        ("base_url", step.get("base_url")),
        ("api_version", step.get("api_version")),
    ):
        if value is not None:
            params[name] = value
    url = COMPLETION.create_azure_base_url(
        azure_client_params=params,
        model=step.get("model"),
        base_model=step.get("base_model"),
    )
    return JSON.validate_python({"url": url})


def url_step(step: Step) -> JsonValue:
    clear_ambient()
    litellm.api_base = step.get("global_base")
    litellm.api_version = step.get("global_version")
    model = step.get("model")
    params: dict[str, JsonValue] = {"api_version": step.get("api_version")}
    try:
        url = EDIT_CONFIG.get_complete_url(
            model="" if model is None else model,
            api_base=step.get("api_base"),
            litellm_params=params,
        )
    except ValueError as error:
        return JSON.validate_python({**FAILURE, "status": None, "message": str(error)})
    return JSON.validate_python({"url": url})


def form_step(step: Step) -> JsonValue:
    data = step.get("data")
    request_url = step.get("request_url")
    fields = EDIT_CONFIG.azure_deployment_image_edit_form_data(
        {} if data is None else data,
        "" if request_url is None else request_url,
    )
    return JSON.validate_python({"fields": fields})


def auth_step(step: Step) -> JsonValue:
    clear_ambient()
    token = step.get("env_token")
    if token is not None:
        os.environ["AZURE_AD_TOKEN"] = token
    supplied = step.get("params")
    given = step.get("headers")
    try:
        headers = EDIT_CONFIG.validate_environment(
            headers={} if given is None else dict(given),
            model="dall-e-3",
            api_key=step.get("api_key"),
            litellm_params={} if supplied is None else dict(supplied),
        )
    except ValueError as error:
        return JSON.validate_python({**FAILURE, "status": None, "message": str(error)})
    return JSON.validate_python({"headers": headers})


def sent_record(transport: Transport, logging: RecordingLogging) -> dict[str, JsonValue]:
    sent = transport.calls[0] if transport.calls else {}
    pre = logging.pre_inputs
    return {
        "sent_url": sent.get("sent_url"),
        "sent_body": sent.get("sent_body"),
        "input": pre[0] if pre else None,
        "pre_calls": len(pre),
        "post_calls": len(logging.posts),
    }


def reply_report(view: ImageReply) -> dict[str, JsonValue]:
    entries = view["data"]
    first = entries[0] if entries else None
    return {
        "created": view["created"],
        "size": None,
        "quality": None,
        "output_format": None,
        "b64_json": None if first is None else first["b64_json"],
        "image_url": None if first is None else first["url"],
        "images": len(entries),
    }


def generation_step(step: Step) -> JsonValue:
    clear_ambient()
    body = step.get("reply_body")
    transport = Transport("" if body is None else body)
    litellm.client_session = httpx.Client(transport=transport)
    logging = RecordingLogging()
    prompt = step.get("prompt")
    optional = step.get("optional")
    given = step.get("headers")
    try:
        response = COMPLETION.image_generation(
            prompt="" if prompt is None else prompt,
            timeout=600.0,
            optional_params={} if optional is None else dict(optional),
            logging_obj=logging,
            headers={} if given is None else dict(given),
            model=step.get("model"),
            api_key=step.get("api_key"),
            api_base=step.get("api_base"),
            api_version=step.get("api_version"),
            azure_ad_token=step.get("azure_ad_token"),
            litellm_params={"base_model": step.get("base_model")},
        )
    except AzureOpenAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46
        return JSON.validate_python({
            **FAILURE,
            "status": error.status_code,
            "message": str(error),
            **sent_record(transport, logging),
        })
    calls = transport.calls
    sent_headers = calls[0]["sent_headers"] if calls else None
    return JSON.validate_python({
        "kind": "response",
        "sent_headers": sent_headers,
        **sent_record(transport, logging),
        "reply": reply_report(REPLY.validate_json(response.model_dump_json())),
    })


def run(argument: Input) -> JsonValue:
    targets = argument.get("targets")
    models = argument.get("models")
    forms = argument.get("forms")
    urls = argument.get("urls")
    auth = argument.get("auth")
    generations = argument.get("generations")
    return JSON.validate_python({
        "targets": [] if targets is None else [target_step(step) for step in targets],
        "models": [] if models is None else [model_step(model) for model in models],
        "forms": [] if forms is None else [form_step(step) for step in forms],
        "urls": [] if urls is None else [url_step(step) for step in urls],
        "auth": [] if auth is None else [auth_step(step) for step in auth],
        "generations": (
            [] if generations is None else [generation_step(step) for step in generations]
        ),
    })
