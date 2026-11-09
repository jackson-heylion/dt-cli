"""离线核对封装参数位置与必填项；完整Schema和权限仍由CLI校验。"""

from dataclasses import dataclass
from typing import List

from cli_result import load_json, read_text


class InputBuildError(ValueError):
    def __init__(self, code, message):
        # 可展示的技术错误码，不包含业务参数值。
        self.code = code
        # 只说明字段位置和恢复方法的中文信息。
        self.message = message
        super().__init__(message)


@dataclass
class ParameterContract:
    # 运行时契约的参数名。
    name: str
    # PATH、QUERY、HEADER或FORM；不根据字段名猜测。
    location: str
    # 此固定版本是否要求该参数。
    required: bool


@dataclass
class InputContract:
    # 已解析的真实非空操作编号。
    operation_id: str
    # 已解析的正整数执行版本。
    operation_version: int
    # 本版本完整具名URL／表单参数清单。
    parameters: List[ParameterContract]
    # 是否必须提供请求正文。
    body_required: bool


def read_contract(file_name):
    value = load_json(read_text(file_name, 2097152))
    if isinstance(value, dict) and 'ok' in value:
        if value.get('ok') is not True or not isinstance(value.get('data'), dict):
            raise InputBuildError('CONTRACT_UNAVAILABLE', '契约解析未成功，不能构造调用输入。')
        value = value['data'].get('contract')
    if not isinstance(value, dict) or not isinstance(value.get('parameters'), list):
        raise InputBuildError('CONTRACT_UNAVAILABLE', '缺少完整执行契约与参数清单。')
    operation_id, version = value.get('operationId'), value.get('operationVersion')
    if not isinstance(operation_id, str) or not operation_id.strip() or type(version) is not int or version < 1:
        raise InputBuildError('CONTRACT_UNAVAILABLE', '契约缺少真实操作编号或正整数版本。')
    parameters = []
    for item in value['parameters']:
        if not isinstance(item, dict) or not isinstance(item.get('name'), str) or not item['name']:
            raise InputBuildError('CONTRACT_UNAVAILABLE', '契约参数没有具名定义。')
        location, required = item.get('location'), item.get('required')
        if location not in ('PATH', 'QUERY', 'HEADER', 'FORM') or type(required) is not bool:
            raise InputBuildError('CONTRACT_UNAVAILABLE', '契约参数位置或必填标记无效。')
        parameters.append(ParameterContract(item['name'], location, required))
    if len({(p.location, p.name) for p in parameters}) != len(parameters):
        raise InputBuildError('CONTRACT_UNAVAILABLE', '契约包含重复位置和参数名。')
    body_required = value.get('requestBodyRequired')
    if type(body_required) is not bool:
        raise InputBuildError('CONTRACT_UNAVAILABLE', '契约缺少请求正文必填标记。')
    return InputContract(operation_id, version, parameters, body_required)


def check_input(contract, invocation):
    definitions = {(p.location, p.name): p for p in contract.parameters}
    provided = {('PATH', p.name) for p in invocation.path} | {('QUERY', p.name) for p in invocation.query}
    for location, name in sorted(provided):
        if (location, name) not in definitions:
            expected = [p.location for p in contract.parameters if p.name == name]
            if expected:
                raise InputBuildError('PARAMETER_LOCATION',
                    '参数 ' + name + ' 应放在 ' + '／'.join(expected) + '，当前位于 ' + location + '。')
            raise InputBuildError('UNKNOWN_PARAMETER', '当前契约未声明 ' + location + ' 参数 ' + name + '。')
    for parameter in contract.parameters:
        if parameter.required and (parameter.location, parameter.name) not in provided:
            raise InputBuildError('REQUIRED_PARAMETER',
                '缺少 ' + parameter.location + ' 必填参数 ' + parameter.name + '。')
    if contract.body_required and invocation.body_json is None:
        raise InputBuildError('REQUIRED_BODY', '当前契约要求JSON正文，构造时须提供body-file。')
