from pydantic import BaseModel, JsonValue


class TestPromptRequest(BaseModel):
    dotprompt_content: str
    prompt_variables: dict[str, JsonValue] | None = None
    conversation_history: list[dict[str, str]] | None = None
