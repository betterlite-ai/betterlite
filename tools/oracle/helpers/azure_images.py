import os
from typing import TypeAlias, TypedDict

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

Json: TypeAlias = JsonValue
SendParams: TypeAlias = dict[str, Json]

JsonObject = TypeAdapter(dict[str, Json])
JsonValueAdapter = TypeAdapter(Json)


class TargetStep(TypedDict, total=False):
    azure_endpoint: str | None
    base_url: str | None
    api_version: str | None
    model: str | None
    base_model: str | None


class FormStep(TypedDict):
    data: dict[str, Json]
    request_url: str


class UrlStep(TypedDict, total=False):
    model: str
    api_base: str | None
    api_version: str | None
    global_base: str | None
    global_version: str | None


class AuthBlock(TypedDict, total=False):
    tenant_id: str | None
    client_id: str | None
    client_secret: str | None
    azure_username: str | None
    azure_password: str | None
    azure_scope: str | None


class Params(TypedDict, total=False):
    api_key: str | None
    azure_ad_token: str | None
    auth: AuthBlock


class AuthStep(TypedDict, total=False):
    headers: dict[str, str]
    api_key: str | None
    params: Params
    env_token: str | None


class ReplyStep(TypedDict, total=False):
    status: int
    body: str


class GenerationStep(TypedDict, total=False):
    prompt: str
    model: str | None
    base_model: str | None
    optional: dict[str, Json]
    headers: dict[str, str]
    api_key: str | None
    api_base: str | None
    api_version: str | None
    azure_ad_token: str | None
    is_async: bool
    reply: ReplyStep


class Input(TypedDict, total=False):
    targets: list[TargetStep]
    models: list[str]
    forms: list[FormStep]
    urls: list[UrlStep]
    auth: list[AuthStep]
    generations: list[GenerationStep]


class ImageReplyData(TypedDict, total=False):
    b64_json: Json
    url: Json


class ImageReplyFields(TypedDict, total=False):
    created: Json
    data: list[ImageReplyData]


ImageReplyAdapter = TypeAdapter(ImageReplyFields)


class Recorder:
    def __init__(self) -> None:
        self.calls: list[SendParams] = []
        self.reply: ReplyStep = {}


RECORDER = Recorder()


def transport_handler(request: httpx.Request) -> httpx.Response:
    headers: dict[str, str] = dict(request.headers)
    body: Json = None
    if request.content:
        body = JsonValueAdapter.validate_json(request.content)
    RECORDER.calls.append(
        {
            "sent_url": str(request.url),
            "sent_body": body,
            "sent_headers": {
                "api-key": headers.get("api-key"),
                "authorization": headers.get("authorization"),
            },
        }
    )
    return httpx.Response(
        status_code=RECORDER.reply.get("status", 200),
        content=RECORDER.reply.get("body", "").encode(),
        request=request,
    )


AMBIENT_KEYS = (
    "AZURE_API_BASE",
    "AZURE_API_VERSION",
    "AZURE_OPENAI_API_KEY",
    "AZURE_API_KEY",
    "AZURE_AD_TOKEN",
)


def clear_ambient() -> None:
    for name in AMBIENT_KEYS:
        os.environ.pop(name, None)
    litellm.api_key = None
    litellm.azure_key = None


def client_params(step: TargetStep) -> SendParams:
    params: SendParams = {}
    if "azure_endpoint" in step:
        params["azure_endpoint"] = step["azure_endpoint"]
    if "base_url" in step:
        params["base_url"] = step["base_url"]
    if "api_version" in step:
        params["api_version"] = step["api_version"]
    return params


def target_step(step: TargetStep) -> SendParams:
    url = AzureChatCompletion().create_azure_base_url(
        azure_client_params=client_params(step),
        model=step.get("model"),
        base_model=step.get("base_model"),
    )
    return {"url": url}


def model_step(model: str) -> SendParams:
    config = get_azure_image_generation_config(model)
    return {"class": config.__class__.__name__}


def form_step(step: FormStep) -> SendParams:
    fields = AzureImageEditConfig.azure_deployment_image_edit_form_data(
        step["data"], step["request_url"]
    )
    return {"fields": JsonObject.validate_python(fields)}


def url_step(step: UrlStep) -> SendParams:
    # the adapter's edit URL sees only the step's credentials, never the process env
    clear_ambient()
    litellm.api_base = step.get("global_base")
    litellm.api_version = step.get("global_version")
    try:
        url = AzureImageEditConfig().get_complete_url(
            model=step["model"],
            api_base=step.get("api_base"),
            litellm_params={"api_version": step.get("api_version")},
        )
    except ValueError as error:
        return failure(error, None)
    return {"url": url}


def auth_params(step: AuthStep) -> SendParams:
    params: SendParams = {}
    if "params" not in step:
        return params
    projected = step["params"]
    if "api_key" in projected:
        params["api_key"] = projected["api_key"]
    if "azure_ad_token" in projected:
        params["azure_ad_token"] = projected["azure_ad_token"]
    if "auth" in projected:
        auth = projected["auth"]
        for name in (
            "tenant_id",
            "client_id",
            "client_secret",
            "azure_username",
            "azure_password",
            "azure_scope",
        ):
            if name in auth:
                params[name] = auth[name]
    return params


def auth_step(step: AuthStep) -> SendParams:
    # the adapter sees the step's environment only, so no ambient key or token may win
    clear_ambient()
    if "env_token" in step:
        os.environ["AZURE_AD_TOKEN"] = str(step.get("env_token"))
    headers: dict[str, str] = {}
    if "headers" in step:
        headers = dict(step["headers"])
    try:
        validated = AzureImageEditConfig().validate_environment(
            headers=headers,
            model="dall-e-3",
            api_key=step.get("api_key"),
            litellm_params=auth_params(step),
        )
    except ValueError as error:
        return failure(error, None)
    return {"headers": JsonObject.validate_python(validated)}


def failure(error: Exception, status: int | None) -> SendParams:
    return {
        "kind": "error",
        "status": status,
        "type": None,
        "param": None,
        "code": None,
        "message": str(error),
    }


def reply_fields(response: ImageResponse) -> SendParams:
    fields = ImageReplyAdapter.validate_json(response.model_dump_json())
    data = fields.get("data", [])
    first: ImageReplyData = data[0] if data else {}
    return {
        "created": fields.get("created"),
        "size": None,
        "quality": None,
        "output_format": None,
        "b64_json": first.get("b64_json"),
        "image_url": first.get("url"),
        "images": len(data),
    }


def generation_step(step: GenerationStep) -> SendParams:
    if step.get("is_async", False):
        raise ValueError("an async generation step has no oracle path")
    clear_ambient()
    RECORDER.calls = []
    RECORDER.reply = step.get("reply", {})
    litellm.client_session = httpx.Client(transport=httpx.MockTransport(transport_handler))
    logging = RecordingLogging()
    optional: dict[str, Json] = {}
    if "optional" in step:
        optional = dict(step["optional"])
    litellm_params: SendParams = {}
    if "base_model" in step:
        litellm_params["base_model"] = step["base_model"]
    headers: dict[str, str] = {}
    if "headers" in step:
        headers = dict(step["headers"])
    try:
        response = AzureChatCompletion().image_generation(
            prompt=required_prompt(step),
            timeout=600.0,
            optional_params=optional,
            logging_obj=logging,
            headers=headers,
            model=step.get("model"),
            api_key=step.get("api_key"),
            api_base=step.get("api_base"),
            api_version=step.get("api_version"),
            azure_ad_token=step.get("azure_ad_token"),
            litellm_params=litellm_params,
        )
    except AzureOpenAIError as error:
        report = failure(error, error.status_code)
        report["sent_url"] = sent("sent_url")
        report["sent_body"] = sent("sent_body")
        report["pre_calls"] = len(logging.pre_inputs)
        report["post_calls"] = len(logging.posts)
        return report
    return {
        "kind": "response",
        "sent_url": sent("sent_url"),
        "sent_body": sent("sent_body"),
        "sent_headers": sent("sent_headers"),
        "input": logging.pre_inputs[0] if logging.pre_inputs else None,
        "pre_calls": len(logging.pre_inputs),
        "post_calls": len(logging.posts),
        "reply": reply_fields(response),
    }


def required_prompt(step: GenerationStep) -> str:
    prompt = step.get("prompt")
    if prompt is None:
        raise ValueError("a generation step needs a prompt")
    return prompt


def sent(name: str) -> Json:
    if not RECORDER.calls:
        return None
    return RECORDER.calls[0].get(name)


class RecordingLogging(Logging):
    def __init__(self) -> None:
        self.pre_inputs: list[str] = []
        self.posts: list[str] = []

    @override
    def pre_call(self, *args: Json, **values: Json) -> None:
        self.pre_inputs.append(str(args[0] if args else values.get("input", "")))

    @override
    def post_call(self, *args: Json, **values: Json) -> None:
        self.posts.append(str(args[0] if args else values.get("input", "")))


def run(argument: Input) -> Json:
    targets: list[Json] = [target_step(step) for step in argument.get("targets", [])]
    models: list[Json] = [model_step(model) for model in argument.get("models", [])]
    forms: list[Json] = [form_step(step) for step in argument.get("forms", [])]
    urls: list[Json] = [url_step(step) for step in argument.get("urls", [])]
    auth: list[Json] = [auth_step(step) for step in argument.get("auth", [])]
    generations: list[Json] = [
        generation_step(step) for step in argument.get("generations", [])
    ]
    return {
        "targets": targets,
        "models": models,
        "forms": forms,
        "urls": urls,
        "auth": auth,
        "generations": generations,
    }
