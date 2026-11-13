"""在已登录的 dt-cli 中解析并调用固定的只读接口。"""

import json
import subprocess
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional, Union

SYSTEM = "dt-smart"


class PlatformError(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class Operation:
    # 平台发布的操作标识及不可变版本。
    identifier: str
    version: int
    # 当前接口允许的请求数上限。
    max_pages: int


@dataclass
class ReadResult:
    # 已读取并验证的业务响应；可为列表或单对象。
    data: Union[list, dict] = field(default_factory=list)
    # 所请求页的覆盖状态、上游分页统计及失败原因。
    pages: int = 0
    complete: bool = False
    total_num: Optional[int] = None
    total_page: Optional[int] = None
    error: Optional[PlatformError] = None


def alias(command):
    return f"shop-operation-{command}"


def integer(value, minimum=0):
    return type(value) is int and value >= minimum


class PlatformClient:
    def __init__(self, launcher, profile, environment, timeout):
        self.launcher = launcher
        self.profile = profile
        self.environment = environment
        self.timeout = timeout
        self.operations = {}

    def invoke(self, *args):
        try:
            result = subprocess.run(
                [self.launcher, *args, "--profile", self.profile],
                capture_output=True, text=True, encoding="utf-8",
                timeout=self.timeout, check=False,
            )
        except FileNotFoundError as exc:
            raise PlatformError("CLI_NOT_FOUND", "找不到 dt-cli，请指定已安装的原生程序路径。") from exc
        except subprocess.TimeoutExpired as exc:
            raise PlatformError("CLI_TIMEOUT", "dt-cli 调用超时，已取得的页仍可使用。") from exc
        try:
            envelope = json.loads(result.stdout)
        except ValueError as exc:
            raise PlatformError("CLI_RESPONSE_INVALID", "dt-cli 未返回可识别的 JSON 信封。") from exc
        if not isinstance(envelope, dict):
            raise PlatformError("CLI_RESPONSE_INVALID", "dt-cli 信封结构不匹配。")
        if result.returncode or envelope.get("ok") is not True:
            error = envelope.get("error") or {}
            raise PlatformError(error.get("code") or "CLI_FAILED",
                                error.get("message") or "dt-cli 请求未完成。")
        data = envelope.get("data")
        if not isinstance(data, dict):
            raise PlatformError("CLI_RESPONSE_INVALID", "dt-cli 信封缺少结果对象。")
        return data

    def resolve(self, command, spec):
        if command in self.operations:
            return self.operations[command]
        data = self.invoke("resolve", "--system", SYSTEM, "--alias", alias(command))
        contract = data.get("contract") or {}
        version = contract.get("operationVersion")
        operation_id = contract.get("operationId")
        limits = contract.get("limits") or {}
        max_pages = limits.get("maxPages")
        minimum_version = spec.get("minimum_stg_version", 1) if self.environment == "stg" else 1
        if (contract.get("systemId") != SYSTEM or contract.get("environment") != self.environment
                or contract.get("path") != spec["path"] or contract.get("method") != "POST"
                or contract.get("effect") != "READ" or contract.get("viewTypes") != []
                or not operation_id or not integer(version, minimum_version) or not data.get("digest")
                or not integer(max_pages, 1)):
            raise PlatformError("CONTRACT_MISMATCH", "平台契约的环境、接口、权限模式或版本与本 Skill 不符。")
        if spec["paged"]:
            paging = contract.get("pagination") or {}
            if (paging.get("kind") != "PAGE" or paging.get("placement") != "JSON_BODY"
                    or paging.get("pageParameter") != "/page" or paging.get("sizeParameter") != "/size"
                    or paging.get("itemsPointer") != "/data" or paging.get("firstPage") != 1):
                raise PlatformError("PAGINATION_CONTRACT_MISMATCH", "平台契约与原接口的正文分页约定不符。")
        operation = Operation(operation_id, version, max_pages)
        self.operations[command] = operation
        return operation

    def call(self, operation, body):
        request = {
            "path": [], "query": [], "headers": [], "form": [],
            "bodyJson": json.dumps(body, ensure_ascii=False), "view": None,
        }
        with tempfile.TemporaryDirectory(prefix="dt-smart-input-") as directory:
            path = Path(directory) / "input.json"
            path.write_text(json.dumps(request, ensure_ascii=False), encoding="utf-8")
            data = self.invoke("call", "--operation", operation.identifier, "--version",
                               str(operation.version), "--input-file", str(path))
        if data.get("businessSuccess") is not True or data.get("contractMatched") is not True:
            raise PlatformError("BUSINESS_CALL_FAILED", "业务响应失败或不符合已发布契约。")
        try:
            result = json.loads(data.get("body"))
        except (ValueError, TypeError) as exc:
            raise PlatformError("BUSINESS_RESPONSE_INVALID", "业务正文不是有效的 JSON。") from exc
        if (not isinstance(result, dict) or result.get("status") != "SUCCESS"
                or result.get("code") != 200 or not isinstance(result.get("data"), (dict, list))):
            raise PlatformError("BUSINESS_RESPONSE_INVALID", "业务信封缺少成功状态或结果。")
        return result

    def read(self, command, spec, body, max_pages):
        operation = self.resolve(command, spec)
        if not spec["paged"]:
            return ReadResult(data=self.call(operation, body)["data"], pages=1, complete=True)
        first_page = body["page"]
        result = ReadResult()
        for page in range(first_page, first_page + min(max_pages, operation.max_pages)):
            try:
                payload = self.call(operation, {**body, "page": page})
                rows, total, last = payload["data"], payload.get("totalNum"), payload.get("totalPage")
                if (not isinstance(rows, list) or not integer(total) or not integer(last)
                        or (not rows and page < last)
                        or (result.pages and (total != result.total_num or last != result.total_page))):
                    raise PlatformError("PAGINATION_RESPONSE_CHANGED", "分页结构或总数已变化，保留此前已验证的页。")
            except PlatformError as exc:
                if not result.pages:
                    raise
                result.error = exc
                break
            result.data.extend(rows)
            result.pages += 1
            result.total_num, result.total_page = total, last
            if page >= last:
                result.complete = first_page == 1
                break
        return result
