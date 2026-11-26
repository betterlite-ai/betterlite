import io
from contextlib import redirect_stdout
from typing import Final, Literal, TypedDict

import litellm
from litellm.llms.base_llm.videos.transformation import BaseVideoConfig
from litellm.types.utils import LlmProviders
from litellm.videos.utils import VideoGenerationRequestUtils
from pydantic import JsonValue, TypeAdapter


class CallInput(TypedDict):
    prompt: str
    model: str | None
    seconds: str | None
    size: str | None
    user: str | None
    custom_llm_provider: str | None
    extra_headers: dict[str, JsonValue] | None
    extra_query: dict[str, JsonValue] | None
    extra_body: dict[str, JsonValue] | None
    timeout: float | None
    kwargs: dict[str, JsonValue]


class Input(TypedDict):
    function: Literal["requested", "provider", "mock"]
    call: CallInput
    model: str
    drop_params: bool
    mock_response: str


PARAMS: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


def run(input_value: Input) -> JsonValue:
    call = input_value["call"]
    if input_value["function"] == "mock":
        with redirect_stdout(io.StringIO()):
            object_value = litellm.video_generation(
                prompt=call["prompt"],
                mock_response=input_value["mock_response"],
                avideo_generation=False,
            )
        return PARAMS.validate_python(object_value.model_dump())  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/videos/main.py#L9
    options = VideoGenerationRequestUtils.get_requested_video_generation_optional_param(
        local_vars(call)
    )
    if input_value["function"] == "requested":
        return PARAMS.validate_python(options)
    model = input_value["model"]
    config: BaseVideoConfig | None = litellm.ProviderConfigManager.get_provider_video_config(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L9691
        model=model, provider=LlmProviders("openai")
    )
    if config is None:
        raise ValueError("openai has no video config")
    return PARAMS.validate_python(
        VideoGenerationRequestUtils.get_optional_params_video_generation(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/utils.py#L13
            model=model,
            video_generation_provider_config=config,
            video_generation_optional_params=options,
        )
    )


# video_generation's locals() before its **kwargs merge over them, the dict the util reads
def local_vars(call: CallInput) -> dict[str, JsonValue]:
    values: dict[str, JsonValue] = {
        "prompt": call["prompt"],
        "model": call["model"],
        "input_reference": None,
        "seconds": call["seconds"],
        "size": call["size"],
        "user": call["user"],
        "timeout": call["timeout"],
        "custom_llm_provider": call["custom_llm_provider"],
        "extra_headers": call["extra_headers"],
        "extra_query": call["extra_query"],
        "extra_body": call["extra_body"],
        "kwargs": call["kwargs"],
    }
    values.update(call["kwargs"])
    return values
