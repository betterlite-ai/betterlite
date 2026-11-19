from types import MethodType
from typing import NotRequired, Protocol, TypedDict

from litellm.llms.custom_llm import CustomLLM, custom_chat_llm_router


class Call(TypedDict):
    async_fn: bool
    stream: NotRequired[bool]


class Input(TypedDict):
    cases: dict[str, Call]


class Router(Protocol):
    def __call__(
        self, *, async_fn: bool, stream: bool | None, custom_llm: CustomLLM
    ) -> MethodType: ...


# the pin declares no return type; custom_chat_llm_router answers one CustomLLM method
router: Router = custom_chat_llm_router


def select(*, async_fn: bool, stream: bool | None) -> str:
    llm = CustomLLM()
    chosen = router(async_fn=async_fn, stream=stream, custom_llm=llm)
    if chosen == router(async_fn=False, stream=False, custom_llm=llm):
        return "completion"
    if chosen == router(async_fn=False, stream=True, custom_llm=llm):
        return "streaming"
    if chosen == router(async_fn=True, stream=False, custom_llm=llm):
        return "acompletion"
    if chosen == router(async_fn=True, stream=True, custom_llm=llm):
        return "astreaming"
    raise ValueError("custom_chat_llm_router returned no CustomLLM method")


def run(input_value: Input) -> dict[str, str]:
    return {
        name: select(async_fn=call["async_fn"], stream=call.get("stream"))
        for name, call in input_value["cases"].items()
    }
