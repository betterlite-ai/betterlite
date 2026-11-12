from typing import cast

from litellm.llms.langflow.chat.transformation import LangFlowConfig

from .operations import flow_value, reply_value, requests_value
from .views import ConfigView, Input, Json


def config_view() -> ConfigView:
    # LiteLLM leaves LangFlowConfig and every method on it unannotated
    return cast("ConfigView", LangFlowConfig())  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langflow/chat/transformation.py#L34-L317


def run(input_value: Input) -> Json:
    config = config_view()
    if input_value["op"] == "flow":
        return flow_value(config, input_value)
    if input_value["op"] == "request":
        return requests_value(config, input_value)
    return {"replies": [reply_value(config, case) for case in input_value["replies"]]}
