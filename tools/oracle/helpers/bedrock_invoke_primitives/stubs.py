import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from datetime import UTC, datetime
from importlib import import_module
from typing import Protocol, cast

from litellm.litellm_core_utils.prompt_templates import huggingface_template_handler
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.chat.invoke_transformations import base_invoke_transformation
from litellm.types.llms.bedrock import AwsAuthParams

from .models import Json, Keys


class DefaultSecret:
    def __call__(self, _name: str, default: str | None = None) -> str | None:
        return default


class BedrockBearer:
    def __init__(self, token: str | None) -> None:
        self.token = token

    def __call__(self, name: str) -> str | None:
        return self.token if name == "AWS_BEARER_TOKEN_BEDROCK" else None


class FixedCredentials:
    def __init__(self, keys: Keys) -> None:
        self.access_key = keys["access_key_id"]
        self.secret_key = keys["secret_access_key"]
        self.token = keys.get("session_token")

    def __call__(self, _auth_params: AwsAuthParams, _region: str | None) -> "FixedCredentials":
        return self


class FrozenClock:
    def __init__(self, now: int) -> None:
        self.frozen = datetime.fromtimestamp(now, tz=UTC).replace(tzinfo=None)

    def __call__(self) -> datetime:
        return self.frozen


class UnreachableTemplate:
    def __call__(self, _hf_model_name: str) -> dict[str, Json]:
        return {"status": "failure"}


class SessionStub:
    def __init__(self) -> None:
        self.region_name: str | None = None


class Boto3Module(Protocol):
    Session: type[SessionStub]


class BotocoreAuthModule(Protocol):
    get_current_datetime: Callable[[], datetime]


class TemplateAcquisition(Protocol):
    _get_tokenizer_config: Callable[[str], dict[str, Json]]
    _get_chat_template_file: Callable[[str], dict[str, Json]]


class SecretsModule(Protocol):
    get_secret: Callable[[str, str | None], str | None]
    get_secret_str: Callable[[str], str | None]


boto3_module = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L777
    "Boto3Module", import_module("boto3")
)
auth_module = cast(  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/auth.py#L33
    "BotocoreAuthModule", import_module("botocore.auth")
)
secrets_module = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L35
    "SecretsModule", import_module("litellm.llms.bedrock.base_aws_llm")
)
templates = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/prompt_templates/huggingface_template_handler.py#L41-L89
    "TemplateAcquisition", huggingface_template_handler
)
guardrail_headers = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L55-L74
    "Callable[[Json], dict[str, str]]",
    base_invoke_transformation._bedrock_invoke_guardrail_headers,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L55-L74
)


@contextmanager
def sealed_environment(environment: dict[str, str]) -> Generator[None]:
    original_env = dict(os.environ)
    original_session = boto3_module.Session
    os.environ.clear()
    os.environ.update(environment)
    boto3_module.Session = SessionStub
    try:
        yield
    finally:
        boto3_module.Session = original_session
        os.environ.clear()
        os.environ.update(original_env)


@contextmanager
def signing_harness(bearer: str | None, credentials: FixedCredentials, now: int) -> Generator[None]:
    original_secret = secrets_module.get_secret
    original_secret_str = secrets_module.get_secret_str
    original_clock = auth_module.get_current_datetime
    original_resolve = BaseAWSLLM.resolve_credentials  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L512
    secrets_module.get_secret = DefaultSecret()
    secrets_module.get_secret_str = BedrockBearer(bearer)
    auth_module.get_current_datetime = FrozenClock(now)
    BaseAWSLLM.resolve_credentials = credentials  # type: ignore[method-assign, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L512
    try:
        with sealed_environment({}):
            yield
    finally:
        secrets_module.get_secret = original_secret
        secrets_module.get_secret_str = original_secret_str
        auth_module.get_current_datetime = original_clock
        BaseAWSLLM.resolve_credentials = original_resolve  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L512


@contextmanager
def unreachable_templates() -> Generator[None]:
    saved = (templates._get_tokenizer_config, templates._get_chat_template_file)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/prompt_templates/huggingface_template_handler.py#L41-L89
    templates._get_tokenizer_config = templates._get_chat_template_file = UnreachableTemplate()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/prompt_templates/huggingface_template_handler.py#L41-L89
    try:
        yield
    finally:
        templates._get_tokenizer_config, templates._get_chat_template_file = saved  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/prompt_templates/huggingface_template_handler.py#L41-L89
