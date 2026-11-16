from litellm.llms.databricks.chat.transformation import DatabricksConfig
from pydantic import JsonValue, TypeAdapter

from .common import Input, MessagesCase, Object, cases_of, chat_api, guarded

CHAT = chat_api(DatabricksConfig())


def messages_of(case: MessagesCase) -> Object:
    body = CHAT.transform_request(case["model"], case["messages"], {}, {}, {})
    return {"messages": body.get("messages")}


def messages_value(input_value: Input) -> JsonValue:
    cases = TypeAdapter(list[MessagesCase]).validate_python(cases_of(input_value))
    return [guarded(messages_of, case) for case in cases]
