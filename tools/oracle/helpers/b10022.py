from typing import TypedDict

from litellm.decisions.call import supported_decisions_providers


class Input(TypedDict):
    pass


def run(_input_value: Input) -> dict[str, list[str]]:
    providers = supported_decisions_providers()
    return {"providers": list(providers)}
