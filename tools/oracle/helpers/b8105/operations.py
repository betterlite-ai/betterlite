import os
import time
from functools import partial

import httpx
import litellm

from .views import (
    ConfigView,
    CreateFileView,
    CreatePlanInput,
    CreateReplyInput,
    DeletedCall,
    FileContentView,
    FileResult,
    FileSpec,
    HeadersAndUrlInput,
    Json,
    ListedCall,
    Object,
    PlanCase,
    PlanView,
    ReplyCase,
    RequestCall,
    RequestsInput,
    Sdk,
    config,
)

MANAGED_ENV = ("MANUS_API_BASE", "MANUS_API_KEY", "TZ")
EMPTY: dict[str, Json] = {}


def activate(environment: dict[str, str]) -> None:
    for name in MANAGED_ENV:
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value
    if "TZ" in environment:
        time.tzset()


def activate_sdk(sdk: Sdk) -> None:
    litellm.api_key = sdk["api_key"]
    litellm.api_base = sdk["api_base"]


def file_view(file: FileResult) -> Json:
    return {
        "id": file.id,
        "bytes": file.bytes,
        "created_at": file.created_at,
        "filename": file.filename,
        "object": file.object,
        "purpose": file.purpose,
        "status": file.status,
        "status_details": file.status_details,
    }


def case_params(api_base: str | None, api_key: str | None) -> Object:
    params: Object = {}
    if api_base is not None:
        params["api_base"] = api_base
    if api_key is not None:
        params["api_key"] = api_key
    return params


def headers_and_url(input_value: HeadersAndUrlInput) -> Json:
    validated: list[Json] = []
    for header_case in input_value["validation"]:
        activate(header_case["environment"])
        activate_sdk(header_case["sdk"])
        try:
            resolved = config().validate_environment(
                headers=dict(header_case["headers"]),
                model="",
                messages=[],
                optional_params=EMPTY,
                litellm_params=EMPTY,
                api_key=header_case["api_key"],
            )
        except ValueError:
            validated.append({"raised": True})
        else:
            validated.append({"headers": resolved})
    urls: list[Json] = []
    for url_case in input_value["urls"]:
        activate(url_case["environment"])
        activate_sdk(url_case["sdk"])
        try:
            url = config().get_complete_url(
                api_base=url_case["api_base"],
                api_key=None,
                model="",
                optional_params=EMPTY,
                litellm_params=EMPTY,
            )
        except ValueError:
            urls.append({"raised": True})
        else:
            urls.append({"url": url})
    return {"validated": validated, "urls": urls}


def create_data(spec: FileSpec) -> CreateFileView:
    if not spec["present"]:
        return {}
    content = bytes(spec["content"])
    filename = spec["filename"]
    supplied = content if filename is None else (filename, content)
    return {"file": supplied, "purpose": "assistants"}


def plan_view(plan: PlanView) -> Json:
    initial = plan["initial_request"]
    upload = plan["upload_request"]
    return {
        "initial": {
            "method": initial["method"],
            "url": initial["url"],
            "headers": initial["headers"],
            "data": initial["data"],
        },
        "upload": {
            "method": upload["method"],
            "url": upload["url"],
            "headers": upload["headers"],
            "data": list(upload["data"]),
        },
        "location": plan["upload_url_location"],
        "key": plan.get("upload_url_key"),
    }


def create_plan(input_value: CreatePlanInput) -> Json:
    activate(input_value["environment"])
    time.time = partial(float, input_value["now"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/manus/files/transformation.py#L264-L268
    client = config()
    return {"plans": [plan_case(client, case) for case in input_value["cases"]]}


def plan_case(client: ConfigView, case: PlanCase) -> Json:
    activate_sdk(case["sdk"])
    params = case_params(case["api_base"], case["api_key"])
    try:
        plan = client.transform_create_file_request(
            model="",
            create_file_data=create_data(case["file"]),
            optional_params=EMPTY,
            litellm_params=params,
        )
    except ValueError:
        return {"raised": True}
    return {"plan": plan_view(plan)}


def create_reply(input_value: CreateReplyInput) -> Json:
    activate(input_value["environment"])
    time.time = partial(float, input_value["now"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/manus/files/transformation.py#L264-L268
    client = config()
    return {"files": [reply_case(client, case) for case in input_value["cases"]]}


def reply_case(client: ConfigView, case: ReplyCase) -> Json:
    response = httpx.Response(200, content=case["body"].encode())
    try:
        file = client.transform_create_file_response(
            model=None,
            raw_response=response,
            logging_obj=None,
            litellm_params=case["litellm_params"],
        )
    except ValueError:
        return {"raised": True}
    return file_view(file)


def request_value(call: RequestCall) -> Json:
    try:
        url, query = call()
    except ValueError:
        return {"raised": True}
    return {"url": url, "params": query}


def deleted_value(call: DeletedCall) -> Json:
    try:
        record = call()
    except ValueError:
        return {"raised": True}
    return {"id": record.id, "object": record.object, "deleted": record.deleted}


def listed_value(call: ListedCall) -> Json:
    try:
        listed = call()
    except ValueError:
        return {"raised": True}
    return [file_view(file) for file in listed]


def file_requests(
    client: ConfigView, file_id: str, litellm_params: Object
) -> tuple[Json, Json, Json]:
    retrieve = partial(
        client.transform_retrieve_file_request,
        file_id=file_id,
        optional_params=EMPTY,
        litellm_params=litellm_params,
    )
    delete = partial(
        client.transform_delete_file_request,
        file_id=file_id,
        optional_params=EMPTY,
        litellm_params=litellm_params,
    )
    content_request: FileContentView = {"file_id": file_id}
    content = partial(
        client.transform_file_content_request,
        file_content_request=content_request,
        optional_params=EMPTY,
        litellm_params=litellm_params,
    )
    return request_value(retrieve), request_value(delete), request_value(content)


def requests(input_value: RequestsInput) -> Json:
    activate(input_value["environment"])
    time.time = partial(float, input_value["now"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/manus/files/transformation.py#L387-L389
    activate_sdk(input_value["sdk"])
    client = config()
    litellm_params: Object = {"api_base": input_value["api_base"]}
    retrieve: list[Json] = []
    delete: list[Json] = []
    content: list[Json] = []
    for file_id in input_value["file_ids"]:
        retrieved, removed, downloaded = file_requests(client, file_id, litellm_params)
        retrieve.append(retrieved)
        delete.append(removed)
        content.append(downloaded)
    deleted = [
        deleted_value(
            partial(
                client.transform_delete_file_response,
                raw_response=httpx.Response(200, content=body.encode()),
                logging_obj=None,
                litellm_params=EMPTY,
            )
        )
        for body in input_value["delete_bodies"]
    ]
    list_requests: list[Json] = []
    for purpose in input_value["purposes"]:
        url, query = client.transform_list_files_request(
            purpose=purpose, optional_params=EMPTY, litellm_params=litellm_params
        )
        list_requests.append({"url": url, "params": query})
    files = listed_value(
        partial(
            client.transform_list_files_response,
            raw_response=httpx.Response(200, content=input_value["body"].encode()),
            logging_obj=None,
            litellm_params=EMPTY,
        )
    )
    return {
        "retrieve": retrieve,
        "delete": delete,
        "deleted": deleted,
        "list_requests": list_requests,
        "files": files,
        "content": content,
    }
