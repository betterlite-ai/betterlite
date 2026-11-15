import litellm

from .wire import Answer, Capabilities

ANSWERS: dict[str, Answer] = {
    "web_search": "no",
    "reasoning": "no",
    "response_schema": "no",
}


class LookupFailureError(Exception):
    pass


class CapabilityAnswer:
    def __init__(self, name: str) -> None:
        self.name = name

    def __call__(self, model: str, custom_llm_provider: str | None = None) -> bool:
        del model, custom_llm_provider
        kind = ANSWERS[self.name]
        if kind == "yes":
            return True
        if kind == "no":
            return False
        raise LookupFailureError(f"the {self.name} capability lookup failed")


def install(capabilities: Capabilities) -> None:
    web_search = capabilities.get("web_search")
    if web_search is not None:
        ANSWERS["web_search"] = web_search
    reasoning = capabilities.get("reasoning")
    if reasoning is not None:
        ANSWERS["reasoning"] = reasoning
    response_schema = capabilities.get("response_schema")
    if response_schema is not None:
        ANSWERS["response_schema"] = response_schema
    litellm.supports_web_search = CapabilityAnswer("web_search")
    litellm.supports_reasoning = CapabilityAnswer("reasoning")
    litellm.supports_response_schema = CapabilityAnswer("response_schema")
