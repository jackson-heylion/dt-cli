"""离线构造开放平台输入；业务条件与权限仍由调用方及CLI核对。"""

import argparse
import json
import math
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import List, Optional, Union

from input_contract import InputBuildError, check_input, read_contract

Scalar = Union[str, int, float, bool]


@dataclass
class Argument:
    # 契约声明的路径或查询参数名。
    name: str
    # URL可传输的标量或标量数组，不接受空值和任意嵌套对象。
    value: Union[Scalar, List[Scalar]]


@dataclass
class TaskView:
    # 当前身份域真实返回的视角编号。
    view_id: int
    # 同一真实候选的对象编号，保持字符串原值。
    object_id: str


@dataclass
class InvocationInput:
    # 根据当前契约填写的路径参数。
    path: List[Argument]
    # 根据当前契约填写的查询参数。
    query: List[Argument]
    # JSON请求体原文；无请求体时为空。
    body_json: Optional[str]
    # 本次固定视角；接口不需要视角时为空。
    view: Optional[TaskView]

    def wire(self):
        # 此处仅组装具名传输模型，不承载业务计算模型。
        return {
            'path': [{'name': a.name, 'value': a.value} for a in self.path],
            'query': [{'name': a.name, 'value': a.value} for a in self.query],
            'headers': [], 'form': [], 'bodyJson': self.body_json,
            'view': None if self.view is None else {
                'viewId': self.view.view_id, 'objectId': self.view.object_id},
        }


def unique_object(pairs):
    result = {}
    for name, value in pairs:
        if name in result:
            raise ValueError('JSON包含重复字段。')
        result[name] = value
    return result


def reject_constant(_value):
    raise ValueError('JSON包含非有限数值。')


def document(file_name, limit):
    path = Path(file_name)
    with path.open('rb') as stream:
        raw = stream.read(limit + 1)
    if len(raw) > limit:
        raise ValueError('输入文件超过传输大小限制。')
    text = raw.decode('utf-8-sig')
    value = json.loads(text, object_pairs_hook=unique_object,
                       parse_constant=reject_constant)
    return text, value


def is_scalar(value):
    return isinstance(value, (str, int, bool)) or (
        isinstance(value, float) and math.isfinite(value))


def arguments(file_name):
    if file_name is None:
        return []
    _, value = document(file_name, 65536)
    if not isinstance(value, dict):
        raise ValueError('路径与查询参数文件必须是具名JSON对象。')
    result = []
    for name, item in value.items():
        if not name or not (is_scalar(item) or isinstance(item, list)
                            and all(is_scalar(v) for v in item)):
            raise ValueError('URL参数须为非空值标量或标量数组。')
        result.append(Argument(name, item))
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description='离线生成平台调用的封装输入文件。')
    parser.add_argument('--path-file', help='具名路径参数JSON文件')
    parser.add_argument('--query-file', help='具名查询参数JSON文件')
    parser.add_argument('--body-file', help='原始JSON正文文件')
    parser.add_argument('--view-id', type=int, help='真实视角编号')
    parser.add_argument('--object-id', help='同一候选的真实对象编号')
    parser.add_argument('--contract-file', help='本次resolve成功文件或完整执行契约；离线核对参数位置与必填项')
    parser.add_argument('--output', required=True, help='尚不存在的输出文件')
    args = parser.parse_args(argv)
    try:
        output = Path(args.output).resolve()
        if output.exists():
            raise InputBuildError('OUTPUT_EXISTS', '输出文件已存在；请使用新文件。本次未生成可用于发送的新输入。')
        if (args.view_id is None) != (args.object_id is None):
            raise ValueError('视角编号与对象编号必须同时提供或同时省略。')
        if args.view_id is not None and (not 0 < args.view_id <= 4294967295
                                         or not args.object_id.strip()):
            raise ValueError('视角与对象必须来自真实非空候选。')
        body = document(args.body_file, 65536)[0] if args.body_file else None
        view = TaskView(args.view_id, args.object_id) if args.view_id is not None else None
        value = InvocationInput(arguments(args.path_file), arguments(args.query_file), body, view)
        contract = read_contract(args.contract_file) if args.contract_file else None
        if contract is not None:
            check_input(contract, value)
        encoded = json.dumps(value.wire(), ensure_ascii=False, allow_nan=False, indent=2) + '\n'
        if len(encoded.encode('utf-8')) > 131072:
            raise ValueError('封装文件超过传输大小限制。')
        with output.open('x', encoding='utf-8', newline='\n') as stream:
            stream.write(encoded)
        print(json.dumps({'ok': True, 'data': {
            'inputFile': str(output), 'arguments': ['--input-file', str(output)],
            'operationId': contract.operation_id if contract else None,
            'operationVersion': contract.operation_version if contract else None}},
            ensure_ascii=False))
        return 0
    except InputBuildError as error:
        print(json.dumps({'ok': False, 'error': {'code': error.code, 'message': error.message}},
                         ensure_ascii=False))
        return 2
    except (OSError, ValueError, UnicodeError):
        # 不输出原始业务内容、参数值、凭证或文件内容。
        print(json.dumps({'ok': False, 'error': {
            'code': 'INPUT_BUILD_FAILED',
            'message': '封装失败，请核对JSON、文件大小、参数类型、成对视角及输出文件是否已存在。'}},
            ensure_ascii=False))
        return 2


if __name__ == '__main__':
    sys.exit(main())
