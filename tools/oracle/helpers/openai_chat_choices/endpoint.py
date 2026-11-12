import json
import os
from typing import cast

import litellm
from litellm.llms.openai.chat.gpt_transformation import OpenAIGPTConfig

from .support import AuthStep, Fields, Json, ModelStep, UrlStep


class Reply:
    def __init__(self, status: int, body: str) -> None:
        self.status_code = status
        self.text = body

    def json(self) -> Json:
        return cast(Json, json.loads(self.text))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L780-L789


class Client:
    def __init__(self, reply: Reply) -> None:
        self.reply = reply
        self.calls: list[tuple[str, dict[str, str]]] = []

    def get(self, url: str, headers: dict[str, str]) -> Reply:
        self.calls.append((url, headers))
        return self.reply


def apply_env(values: dict[str, str]) -> None:
    os.environ.clear()
    os.environ.update(values)


def model_list(steps: list[ModelStep]) -> Json:
    config = OpenAIGPTConfig()
    outputs: list[Json] = []
    for step in steps:
        scripted = step.get("reply")
        environment = step.get("env")
        if scripted is None or environment is None:
            raise ValueError("a model list step needs a reply and an env")
        client = Client(Reply(scripted["status"], scripted["body"]))
        litellm.module_level_client = client  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L777-L779
        apply_env(environment)
        try:
            models = config.get_models(
                api_key=step.get("api_key"), api_base=step.get("api_base")
            )
        except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L782-L783
            outputs.append({"ok": False})
        else:
            outputs.append({
                "ok": True,
                "models": cast(Json, models),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L788-L789
                "url": client.calls[0][0],
                "authorization": client.calls[0][1].get("Authorization"),
            })
    return outputs


def url_join(steps: list[UrlStep]) -> Json:
    config = OpenAIGPTConfig()
    return [
        {"url": config.get_complete_url(step.get("api_base"), None, "m", {}, {})}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L717-L743
        for step in steps
    ]


def auth(steps: list[AuthStep]) -> Json:
    config = OpenAIGPTConfig()
    outputs: list[Json] = []
    for step in steps:
        headers = config.validate_environment(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L745-L762
            dict(step["headers"]), "m", [], {}, {}, api_key=step.get("api_key")  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L745-L762
        )
        outputs.append(cast(Json, headers))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L745-L762
    return outputs


def credentials(fields: Fields) -> Json:
    config = OpenAIGPTConfig()
    environment = fields.get("env")
    model = fields.get("model")
    if environment is None or model is None:
        raise ValueError("credential accessor defaults need an env and a model")
    apply_env(environment)
    litellm.api_key = fields.get("litellm_api_key")
    litellm.openai_key = fields.get("litellm_openai_key")
    litellm.api_base = fields.get("litellm_api_base")
    api_key = config.get_api_key(fields.get("api_key"))
    api_base = config.get_api_base(fields.get("api_base"))
    base_model = config.get_base_model(model)
    counter = config.get_token_counter()
    if counter is None:
        raise RuntimeError("get_token_counter returned no counter")
    kind = "openai_api" if counter.__class__.__name__ == "OpenAITokenCounter" else "other"
    return {
        "api_key": api_key,
        "api_base": api_base,
        "base_model": base_model,
        "token_counter": kind,
    }
