import os
from typing import Final, Protocol, TypeAlias, TypedDict

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

JSON_VALUE = TypeAdapter(Json)


class Step(TypedDict, total=False):
    azure_endpoint: str | None
    base_url: str | None
    api_base: str | None
    api_version: str | None
    model: str | None
    base_model: str | None
    data: dict[str, Json]
    request_url: str
    global_base: str | None
    global_version: str | None
    headers: dict[str, str]
    api_key: str | None
    params: dict[str, Json]
    env_token: str | None
    prompt: str
    optional: dict[str, Json]
    azure_ad_token: str | None
    reply_body: str


class Input(TypedDict, total=False):
    targets: list[Step]
    models: list[str]
    forms: list[Step]
    urls: list[Step]
    auth: list[Step]
    generations: list[Step]


# LiteLLM types both edit helpers with bare dicts, whose Any values no strict checker accepts
class EditConfig(Protocol):
    @staticmethod
    def azure_deployment_image_edit_form_data(
        data: dict[str, Json], request_url: str
    ) -> dict[str, Json]: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        api_key: str | None = None,
        litellm_params: dict[str, Json] | None = None,
    ) -> dict[str, Json]: ...


EDIT_CONFIG: Final[EditConfig] = AzureImageEditConfig()
EDIT_FORM_DATA = EDIT_CONFIG.azure_deployment_image_edit_form_data


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


CALLS: list[SendParams] = []
FAILURE: SendParams = {"kind": "error", "type": None, "param": None, "code": None}
REPLY: dict[str, str] = {}


def clear_ambient() -> None:
    for name in [key for key in os.environ if key.startswith("AZURE_")]:
        os.environ.pop(name, None)
    litellm.api_key = litellm.azure_key = litellm.api_base = litellm.api_version = None


def transport_handler(request: httpx.Request) -> httpx.Response:
    body: Json = JSON_VALUE.validate_json(request.content) if request.content else None
    headers: Json = {
        "api-key": request.headers.get("api-key"),
        "authorization": request.headers.get("authorization"),
    }
    CALLS.append({"sent_url": str(request.url), "sent_body": body, "sent_headers": headers})
    return httpx.Response(status_code=200, content=REPLY.get("body", "").encode(), request=request)


def failure(error: Exception, status: int | None) -> SendParams:
    return {**FAILURE, "status": status, "message": str(error)}


def target_step(step: Step) -> SendParams:
    params: SendParams = {}
    for name, value in (
        ("azure_endpoint", step.get("azure_endpoint")),
        ("base_url", step.get("base_url")),
        ("api_version", step.get("api_version")),
    ):
        if value is not None:
            params[name] = value
    url = AzureChatCompletion().create_azure_base_url(
        azure_client_params=params,
        model=step.get("model"),
        base_model=step.get("base_model"),
    )
    return {"url": url}


def url_step(step: Step) -> SendParams:
    clear_ambient()
    litellm.api_base = step.get("global_base")
    litellm.api_version = step.get("global_version")
    try:
        url = AzureImageEditConfig().get_complete_url(
            model=step.get("model", ""),
            api_base=step.get("api_base"),
            litellm_params={"api_version": step.get("api_version")},
        )
    except ValueError as error:
        return failure(error, None)
    return {"url": url}


def auth_step(step: Step) -> SendParams:
    clear_ambient()
    if "env_token" in step:
        os.environ["AZURE_AD_TOKEN"] = str(step.get("env_token"))
    params: SendParams = dict(step["params"]) if "params" in step else {}
    headers: dict[str, str] = dict(step["headers"]) if "headers" in step else {}
    try:
        validated = EDIT_CONFIG.validate_environment(
            headers=headers,
            model="dall-e-3",
            api_key=step.get("api_key"),
            litellm_params=params,
        )
    except ValueError as error:
        return failure(error, None)
    return {"headers": validated}


def generation_step(step: Step) -> SendParams:
    clear_ambient()
    CALLS.clear()
    REPLY["body"] = step.get("reply_body", "")
    litellm.client_session = httpx.Client(transport=httpx.MockTransport(transport_handler))
    logging = RecordingLogging()
    try:
        response: ImageResponse = AzureChatCompletion().image_generation(
            prompt=step.get("prompt", ""),
            timeout=600.0,
            optional_params=dict(step.get("optional", {})),
            logging_obj=logging,
            headers=dict(step.get("headers", {})),
            model=step.get("model"),
            api_key=step.get("api_key"),
            api_base=step.get("api_base"),
            api_version=step.get("api_version"),
            azure_ad_token=step.get("azure_ad_token"),
            litellm_params={"base_model": step.get("base_model")},
        )
    except AzureOpenAIError as error:
        report = failure(error, error.status_code)
        report["sent_url"] = CALLS[0].get("sent_url") if CALLS else None
        report["sent_body"] = CALLS[0].get("sent_body") if CALLS else None
        report["input"] = logging.pre_inputs[0] if logging.pre_inputs else None
        report["pre_calls"] = len(logging.pre_inputs)
        report["post_calls"] = len(logging.posts)
        return report
    data = response.data or []
    first = data[0] if data else None
    return {
        "kind": "response",
        "sent_url": CALLS[0].get("sent_url") if CALLS else None,
        "sent_body": CALLS[0].get("sent_body") if CALLS else None,
        "sent_headers": CALLS[0].get("sent_headers") if CALLS else None,
        "input": logging.pre_inputs[0] if logging.pre_inputs else None,
        "pre_calls": len(logging.pre_inputs),
        "post_calls": len(logging.posts),
        "reply": {
            "created": response.created,
            "size": None,
            "quality": None,
            "output_format": None,
            "b64_json": first.b64_json if first else None,
            "image_url": first.url if first else None,
            "images": len(data),
        },
    }


def run(argument: Input) -> Json:
    targets: list[Json] = [target_step(step) for step in argument.get("targets", [])]
    models: list[Json] = [
        {"class": get_azure_image_generation_config(model).__class__.__name__}
        for model in argument.get("models", [])
    ]
    forms: list[Json] = [
        {"fields": EDIT_FORM_DATA(step.get("data", {}), step.get("request_url", ""))}
        for step in argument.get("forms", [])
    ]
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
