import httpx
import litellm
import openai
from litellm.llms.azure.chat.gpt_5_transformation import AzureOpenAIGPT5Config
from pydantic import JsonValue

from .configs import GPT5, O_SERIES_STREAM, UNSUPPORTED, Failure, ParamsConfig, Recorder
from .views import (
    AZURE_ENDPOINT,
    AZURE_KEY,
    AZURE_VERSION,
    PROJECTED,
    AliasStep,
    Install,
    MapStep,
    StreamStep,
    TransformStep,
)


def install(source: Install) -> None:
    litellm.model_cost = source["model_cost"]
    litellm.open_ai_chat_completion_models = set(source["chat_models"])
    litellm.open_ai_text_completion_models = set(source["text_models"])
    litellm.model_list_set = set(source["chat_models"]) | set(source["text_models"])
    litellm.drop_params = source["drop_params"]
    litellm.openai_system_messages_first = source["openai_system_messages_first"]


def alias_value(step: AliasStep) -> JsonValue:
    model = step["model"]
    supported: list[JsonValue] = [*GPT5.get_supported_openai_params(model)]
    return {
        "model": model,
        "family": AzureOpenAIGPT5Config.is_model_gpt_5_model(model),
        "supported": supported,
        "resolves_none": AzureOpenAIGPT5Config.effort_resolves_to_none(model, None),
    }


def stream_value(step: StreamStep) -> JsonValue:
    fake = O_SERIES_STREAM.should_fake_stream(
        model=step["model"],
        stream=step["stream"],
        custom_llm_provider=step["provider"],
    )
    return {"model": step["model"], "stream": step["stream"], "fake": fake}


def allowlist_value(config: ParamsConfig, model: str) -> JsonValue:
    supported: list[JsonValue] = [*config.get_supported_openai_params(model)]
    return {"model": model, "supported": supported}


def map_value(step: MapStep) -> JsonValue:
    litellm.drop_params = step["global_drop_params"]
    optional: dict[str, JsonValue] = dict(step["optional"])
    try:
        mapped = GPT5.map_openai_params(
            dict(step["non_default"]),
            optional,
            step["model"],
            drop_params=step["drop_params"],
        )
    except UNSUPPORTED as error:
        failure = Failure.model_validate(error, from_attributes=True)
        return {"ok": False, "value": None, "status": failure.status_code}
    return {"ok": True, "value": mapped, "status": None}


def transform_value(step: TransformStep) -> JsonValue:
    recorder = Recorder()
    client = openai.AzureOpenAI(
        api_key=AZURE_KEY,
        api_version=AZURE_VERSION,
        azure_endpoint=AZURE_ENDPOINT,
        http_client=httpx.Client(transport=httpx.MockTransport(recorder.handle)),
    )
    try:
        litellm.completion(
            model=f"azure/{step['model']}",
            messages=step["messages"],
            api_key=AZURE_KEY,
            api_base=AZURE_ENDPOINT,
            api_version=AZURE_VERSION,
            client=client,
            tools=step["tools"],
            tool_choice=step["tool_choice"],
            reasoning_effort=step["reasoning_effort"],
        )
    except UNSUPPORTED as error:
        failure = Failure.model_validate(error, from_attributes=True)
        return {"ok": False, "value": None, "status": failure.status_code}
    projected: dict[str, JsonValue] = {}
    for name in PROJECTED:
        if name in recorder.body:
            projected[name] = recorder.body[name]
    return {"ok": True, "value": projected, "status": None}
