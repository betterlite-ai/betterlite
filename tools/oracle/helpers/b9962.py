from typing import Literal, NotRequired, Protocol, TypedDict, cast

from litellm.constants import known_tokenizer_config
from litellm.litellm_core_utils.prompt_templates import huggingface_template_handler as templates
from litellm.llms.bedrock.chat.invoke_transformations.base_invoke_transformation import (
    AmazonInvokeConfig,
)
from pydantic import JsonValue


class Case(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    provider: NotRequired[str | None]
    hf_model: str
    tokenizer_config: NotRequired[dict[str, JsonValue]]
    template_file: NotRequired[str]


class Input(TypedDict):
    cases: list[Case]


class TemplateAnswer(TypedDict):
    status: Literal["success", "failure"]
    tokenizer: NotRequired[dict[str, JsonValue]]
    chat_template: NotRequired[str]


class InvokeApi(Protocol):
    def convert_messages_to_prompt(
        self,
        model: str,
        messages: list[dict[str, JsonValue]],
        provider: str | None,
        custom_prompt_dict: dict[str, dict[str, JsonValue]],
    ) -> tuple[str, list[JsonValue] | None]: ...


class CaseTemplates:
    def __init__(self, case: Case) -> None:
        self.case = case

    def __call__(self, hf_model_name: str) -> TemplateAnswer:
        if hf_model_name != self.case["hf_model"]:
            return {"status": "failure"}
        tokenizer_config = self.case.get("tokenizer_config")
        template_file = self.case.get("template_file")
        answer: TemplateAnswer = {"status": "failure"}
        if tokenizer_config is not None:
            answer["tokenizer"] = tokenizer_config
        if template_file is not None:
            answer["chat_template"] = template_file
        if template_file is not None or (
            tokenizer_config is not None and "chat_template" in tokenizer_config
        ):
            answer["status"] = "success"
        return answer


def run_case(invoke: InvokeApi, case: Case) -> JsonValue:
    known_tokenizer_config.clear()
    saved = (templates._get_tokenizer_config, templates._get_chat_template_file)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/prompt_templates/huggingface_template_handler.py#L41-L114
    templates._get_tokenizer_config = templates._get_chat_template_file = CaseTemplates(case)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/prompt_templates/huggingface_template_handler.py#L41-L114
    try:
        prompt, chat_history = invoke.convert_messages_to_prompt(
            case["model"], case["messages"], case.get("provider"), {}
        )
    except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L592-L630
        return {"message": str(error)}
    finally:
        templates._get_tokenizer_config, templates._get_chat_template_file = saved  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/prompt_templates/huggingface_template_handler.py#L41-L114
    return {"prompt": prompt, "chat_history": chat_history}


def run(input_value: Input) -> JsonValue:
    invoke = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L78-L80
        "InvokeApi",
        AmazonInvokeConfig(),  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L78-L80
    )
    results = [run_case(invoke, case) for case in input_value["cases"]]
    return {"results": results}
