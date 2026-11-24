from collections.abc import Mapping, Sequence
from typing import Final

import betterlite
from pydantic import JsonValue

from betterlite.exceptions import UnsupportedParamsError

MESSAGE_KEYS: Final[tuple[str, ...]] = ("role", "content", "attachments", "metadata")
IMAGE_KEYS: Final[tuple[str, ...]] = (
    "n", "quality", "response_format", "size", "style", "user",
)
BEDROCK_IMAGE_PARAMS: Final[tuple[str, ...]] = ("size",)
VERTEX_IMAGE_PARAMS: Final[tuple[str, ...]] = ("n",)
SIZE_PARTS: Final = 2
OPENAI_COMPATIBLE_PROVIDERS: Final[tuple[str, ...]] = (
    "anyscale", "groq", "nvidia_nim", "cerebras", "baseten", "sambanova",
    "ai21_chat", "ai21", "volcengine", "codestral", "deepseek", "tencent",
    "deepinfra", "perplexity", "xinference", "xai", "zai", "together_ai",
    "fireworks_ai", "empower", "friendliai", "azure_ai", "github", "litellm_proxy",
    "hosted_vllm", "llamafile", "lm_studio", "galadriel", "github_copilot", "chatgpt",
    "novita", "meta_llama", "publicai", "synthetic", "tensormesh", "apertis",
    "nano-gpt", "poe", "chutes", "parasail", "libertai", "featherless_ai",
    "nscale", "nebius", "dashscope", "qwencloud", "qwen_ai_platform", "modelscope",
    "moonshot", "v0", "helicone", "morph", "lambda_ai", "inception",
    "hyperbolic", "vercel_ai_gateway", "aiml", "edenai", "wandb", "cometapi",
    "clarifai", "docker_model_runner", "ragflow", "pinstripes", "darkbloom", "meta",
    "cognition", "cortecs", "scx-ai", "prism", "sail",
)
AZURE_CONTENT: Final = "Azure only accepts content as a string."
AZURE_ATTACHMENT: Final = (
    "Azure doesn't support {value}. To drop it from the call, set `litellm.drop_params = True."
)
AZURE_ATTACHMENTS: Final = (
    "Invalid param. attachments should always be a list. "
    "Got={kind}, Expected=List. Raw value={value}"
)
IMAGE_UNSUPPORTED: Final = (
    "Setting user/encoding format is not supported by {provider}. "
    "To drop it from the call, set `litellm.drop_params = True`."
)


def get_optional_params_add_message(  # ast-grep-ignore: no-long-names
    role: JsonValue,
    content: JsonValue,
    attachments: JsonValue,
    metadata: JsonValue,
    custom_llm_provider: str,
    **kwargs: JsonValue,
) -> dict[str, JsonValue]:
    passed: dict[str, JsonValue] = {
        "role": role,
        "content": content,
        "attachments": attachments,
        "metadata": metadata,
        **kwargs,
    }
    selected: dict[str, JsonValue] = {
        name: value
        for name, value in passed.items()
        if name in MESSAGE_KEYS and value is not None
    }
    optional: dict[str, JsonValue] = {}
    if custom_llm_provider == "openai":
        optional = selected
    elif custom_llm_provider == "azure":
        optional = azure_message_params(selected)
    for name, value in passed.items():
        if name not in MESSAGE_KEYS:
            optional[name] = value
    return optional


# azure's supported params are exactly the four defaults, so its check never drops or raises
def azure_message_params(selected: Mapping[str, JsonValue]) -> dict[str, JsonValue]:
    optional: dict[str, JsonValue] = {}
    for name, value in selected.items():
        if name == "content":
            match value:
                case str() as text:
                    optional["content"] = text
                case object():
                    raise UnsupportedParamsError(message=AZURE_CONTENT)
        elif name == "attachments":
            check_azure_attachments(value)
        else:
            optional[name] = value
    return optional


# Azure never sends attachments; it only refuses shapes it cannot turn into the old file_ids param
def check_azure_attachments(value: JsonValue) -> None:
    match value:
        case list() as items:
            for item in items:
                if not is_file_attachment(item) and betterlite.drop_params is not True:
                    raise UnsupportedParamsError(
                        message=AZURE_ATTACHMENT.format(value=value)
                    )
        case object():
            raise UnsupportedParamsError(
                message=AZURE_ATTACHMENTS.format(kind=type(value), value=value)
            )


def is_file_attachment(item: JsonValue) -> bool:
    match item:
        case {"file_id": object()}:
            return True
        case object():
            return False


def get_optional_params_image_gen(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/utils.py#L69
    n: JsonValue = None,
    quality: JsonValue = None,
    response_format: JsonValue = None,
    size: JsonValue = None,
    style: JsonValue = None,
    user: JsonValue = None,
    custom_llm_provider: str | None = None,
    **kwargs: JsonValue,
) -> dict[str, JsonValue]:
    passed: dict[str, JsonValue] = {
        "n": n,
        "quality": quality,
        "response_format": response_format,
        "size": size,
        "style": style,
        "user": user,
        **kwargs,
    }
    selected: dict[str, JsonValue] = {
        name: value
        for name, value in passed.items()
        if name in IMAGE_KEYS and value is not None
    }
    optional: dict[str, JsonValue] = {}
    if custom_llm_provider in ("openai", "azure", *OPENAI_COMPATIBLE_PROVIDERS):
        optional = selected
    elif custom_llm_provider == "bedrock":
        check_image_params(selected, BEDROCK_IMAGE_PARAMS, custom_llm_provider)
        bedrock_size(size, optional)
    elif custom_llm_provider == "vertex_ai":
        check_image_params(selected, VERTEX_IMAGE_PARAMS, custom_llm_provider)
        vertex_count(n, optional)
    for name, value in passed.items():
        if name not in IMAGE_KEYS:
            optional[name] = value
    return optional


def check_image_params(
    selected: dict[str, JsonValue], supported: Sequence[str], provider: str | None
) -> None:
    for name in list(selected):
        if betterlite.drop_params is True and name not in supported:
            del selected[name]
        elif name not in supported:
            raise UnsupportedParamsError(message=IMAGE_UNSUPPORTED.format(provider=provider))


def bedrock_size(size: JsonValue, optional: dict[str, JsonValue]) -> None:
    match size:
        case None:
            return
        case str() as text:
            parts = text.split("x")
            if len(parts) != SIZE_PARTS:
                raise ValueError("size needs a width and a height")
            optional["width"] = int(parts[0])
            optional["height"] = int(parts[1])
        case object():
            raise TypeError("size is not text")


def vertex_count(n: JsonValue, optional: dict[str, JsonValue]) -> None:
    match n:
        case None:
            return
        case bool() | int() | float() | str():
            optional["sampleCount"] = int(n)
        case object():
            raise TypeError("n is not a number")
