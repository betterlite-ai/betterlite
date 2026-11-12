from litellm.exceptions import UnsupportedParamsError
from litellm.integrations.websearch_interception.tools import (
    is_anthropic_native_web_search_tool,
)
from litellm.llms.anthropic import common_utils
from litellm.llms.anthropic.common_utils import (
    eager_input_streaming_flag,
    requires_native_compaction_beta,
    supports_anthropic_cache_control,
)
from litellm.types.llms.anthropic_tool_search import get_tool_search_beta_header
from pydantic import JsonValue as Json

from .contracts import (
    BETA_LIST,
    MODEL_INFO,
    ONE_SHOT,
    OUTPUT_CONFIG,
    PROGRAMMATIC_PREDICATE,
    THINKING_DISPLAY,
    TOOL_CHANGE,
    USER_HEADERS,
    BetaCall,
    BillingFieldsCall,
    CacheCall,
    Call,
    CompactionCall,
    ComputerCall,
    ConversationCall,
    EagerFlagCall,
    EagerUsedCall,
    FileIdsCall,
    OneShotCall,
    PdfCall,
    RequestCall,
    SkillsCall,
    ToolCall,
    ToolsCall,
)


def one_shot_answer(call: OneShotCall) -> Json:
    requested = ONE_SHOT(
        call["messages"], call["system"], call["tools"], call["user_agent"]
    )
    return {"one_shot": requested}


def billing_fields_answer(call: BillingFieldsCall) -> Json:
    answers: list[Json] = []
    for text in call["texts"]:
        parsed = common_utils._claude_code_billing_fields(text)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L144
        if parsed is None:
            answers.append(None)
            continue
        pairs: list[Json] = [[key, value] for key, value in parsed]
        answers.append(pairs)
    return {"fields": answers}


def compaction_answer(call: CompactionCall) -> Json:
    native = requires_native_compaction_beta(
        call["provider"], call["params"], call["messages"]
    )
    return {"native": native}


def cache_answer(call: CacheCall) -> Json:
    supported = supports_anthropic_cache_control(
        call["model"], call["custom_llm_provider"]
    )
    return {"supported": supported}


def file_ids_answer(call: FileIdsCall) -> Json:
    try:
        used: bool | None = MODEL_INFO.is_file_id_used(call["messages"])
    except (AttributeError, KeyError, TypeError, ValueError):
        used = None
    return {"file_id": used}


def pdf_answer(call: PdfCall) -> Json:
    return {
        "pdf": MODEL_INFO.is_pdf_used(call["messages"]),
    }


def tools_answer(call: ToolsCall) -> Json:
    native: list[Json] = []
    if call["tools"] is not None:
        native.extend(
            is_anthropic_native_web_search_tool(tool) for tool in call["tools"]
        )
    return {
        "tool_search": MODEL_INFO.is_tool_search_used(call["tools"]),
        "programmatic": PROGRAMMATIC_PREDICATE(call["tools"]),
        "input_examples": MODEL_INFO.is_input_examples_used(call["tools"]),
        "native": native,
    }


def eager_flag_answer(call: EagerFlagCall) -> Json:
    try:
        flag = eager_input_streaming_flag(call["tool"])
    except UnsupportedParamsError:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
        return {"error": {"status": 400}}
    return {"flag": flag}


def eager_used_answer(call: EagerUsedCall) -> Json:
    try:
        used = MODEL_INFO.is_eager_input_streaming_used(call["tools"])
    except UnsupportedParamsError:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
        return {"error": {"status": 400}}
    return {"used": used}


def computer_answer(call: ComputerCall) -> Json:
    betas: list[Json] = [
        MODEL_INFO.get_computer_tool_beta_header(version)
        for version in call["versions"]
    ]
    return {
        "computer_tool": MODEL_INFO.is_computer_tool_used(call["tools"]),
        "betas": betas,
        "web_search": MODEL_INFO.is_web_search_tool_used(call["tools"]),
    }


def skills_answer(call: SkillsCall) -> Json:
    user_betas: list[Json] = []
    for header in call["headers"]:
        parsed = USER_HEADERS(header)
        if parsed is None:
            user_betas.append(None)
            continue
        parts: list[Json] = []
        parts.extend(parsed)
        user_betas.append(parts)
    search_betas: list[Json] = [
        get_tool_search_beta_header(provider) for provider in call["providers"]
    ]
    return {
        "code_execution": MODEL_INFO.is_code_execution_tool_used(call["tools"]),
        "container_skills": MODEL_INFO.is_container_with_skills_used(call["params"]),
        "user_betas": user_betas,
        "tool_search_betas": search_betas,
    }


def conversation_answer(call: ConversationCall) -> Json:
    return {
        "thinking_display": THINKING_DISPLAY(call["thinking"]),
        "tool_change": TOOL_CHANGE(call["messages"]),
        "output_config": OUTPUT_CONFIG(call["messages"]),
        "mcp": MODEL_INFO.is_mcp_server_used(call["mcp_servers"]),
    }


def beta_answer(call: BetaCall) -> Json:
    betas = BETA_LIST(
        call["model"],
        call["params"],
        call["computer_tool_used"],
        prompt_caching_set=call["prompt_caching_set"],
        file_id_used="file_id" in call["features"],
        mcp_server_used="mcp_server" in call["features"],
        custom_llm_provider=call["custom_llm_provider"],
        is_mid_conversation_output_config_used="output_config" in call["features"],
        is_thinking_display_updates_used="thinking_display" in call["features"],
        is_mid_conversation_tool_change_used="tool_change" in call["features"],
    )
    ordered: list[Json] = []
    ordered.extend(sorted(betas))
    return {"betas": ordered}


def request_answer(call: RequestCall) -> Json:
    match call["function"]:
        case "one_shot":
            return one_shot_answer(call)
        case "billing_fields":
            return billing_fields_answer(call)
        case "compaction":
            return compaction_answer(call)
        case "cache_control":
            return cache_answer(call)
        case "file_ids":
            return file_ids_answer(call)
        case "pdf":
            return pdf_answer(call)


def tool_answer(call: ToolCall) -> Json:
    if call["function"] == "tools":
        return tools_answer(call)
    if call["function"] == "eager_flag":
        return eager_flag_answer(call)
    if call["function"] == "eager_used":
        return eager_used_answer(call)
    if call["function"] == "computer":
        return computer_answer(call)
    if call["function"] == "skills":
        return skills_answer(call)
    return conversation_answer(call)


def answer(call: Call) -> Json:
    match call["function"]:
        case (
            "one_shot"
            | "billing_fields"
            | "compaction"
            | "cache_control"
            | "file_ids"
            | "pdf"
        ):
            return request_answer(call)
        case (
            "tools"
            | "eager_flag"
            | "eager_used"
            | "computer"
            | "skills"
            | "conversation"
        ):
            return tool_answer(call)
        case "beta":
            return beta_answer(call)
