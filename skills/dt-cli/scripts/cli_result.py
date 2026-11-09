"""解析CLI的最终信封；兼容单个JSON和事件行后跟结果的JSONL。"""

import argparse
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Optional


@dataclass
class CliResult:
    # 最终信封原文，保留正文字符串及原始数值精度。
    envelope_json: str
    # CLI命令是否成功；业务读取还须核对后两个可空字段。
    ok: bool
    # 业务是否成功；非业务命令可能不提供。
    business_success: Optional[bool]
    # 响应是否匹配契约；非业务命令可能不提供。
    contract_matched: Optional[bool]
    # 原错误码；成功或未提供时为空，不保存错误中的业务值。
    error_code: Optional[str]
    # 原写入阶段；inspect等命令可能不提供。
    stage: Optional[str]
    # 原业务JSON正文；无正文时为空。
    body_json: Optional[str]

    @property
    def succeeded(self):
        return self.ok and self.business_success is not False and self.contract_matched is not False


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('JSON包含重复字段。')
        result[key] = value
    return result


def reject_constant(_value):
    raise ValueError('JSON包含非有限数值。')


def load_json(text):
    return json.loads(text, object_pairs_hook=unique_object, parse_constant=reject_constant)


def is_envelope(value):
    return isinstance(value, dict) and type(value.get('ok')) is bool and (
        'data' in value or 'error' in value)


def parse_cli_output(text):
    raw = text.lstrip('\ufeff').strip()
    try:
        value = load_json(raw)
    except ValueError:
        # JSONL中的事件不是执行结果；逐行校验后只取最终信封。
        candidate = None
        for line in raw.splitlines():
            if not line.strip():
                continue
            item = load_json(line)
            if not isinstance(item, dict):
                raise ValueError('CLI输出行不是JSON对象。')
            if is_envelope(item):
                candidate = (line.strip(), item)
        if candidate is None:
            raise ValueError('CLI输出缺少最终信封。')
        raw, value = candidate
    if not is_envelope(value):
        raise ValueError('CLI输出缺少最终信封。')
    data = value.get('data')
    if not isinstance(data, dict):
        data = {}
    error = value.get('error')
    if not isinstance(error, dict):
        error = {}
    for key in ('businessSuccess', 'contractMatched'):
        if data.get(key) is not None and type(data[key]) is not bool:
            raise ValueError('CLI成功标记类型不正确。')
    body = data.get('body')
    if body is not None:
        if not isinstance(body, str):
            raise ValueError('CLI正文不是JSON字符串。')
        load_json(body)
    return CliResult(raw, value['ok'], data.get('businessSuccess'),
                     data.get('contractMatched'), error.get('code'), data.get('stage'), body)


def read_text(path, limit=16777216):
    with Path(path).open('rb') as stream:
        raw = stream.read(limit + 1)
    if len(raw) > limit:
        raise ValueError('文件超过读取上限。')
    return raw.decode('utf-8-sig')


def write_new(path, text):
    with Path(path).open('x', encoding='utf-8', newline='\n') as stream:
        stream.write(text.rstrip('\n') + '\n')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', required=True, help='CLI完整标准输出文件')
    parser.add_argument('--output', required=True, help='新的最终信封文件')
    parser.add_argument('--body-output', help='新的业务正文文件；原信封须有正文')
    args = parser.parse_args(argv)
    try:
        result = parse_cli_output(read_text(args.input))
        targets = [Path(args.output)] + ([Path(args.body_output)] if args.body_output else [])
        if any(p.exists() for p in targets) or len({p.resolve() for p in targets}) != len(targets):
            raise ValueError('输出文件已存在或重叠。')
        if args.body_output and result.body_json is None:
            raise ValueError('原信封没有业务正文。')
        write_new(args.output, result.envelope_json)
        if args.body_output:
            write_new(args.body_output, result.body_json)
        print(json.dumps({'ok': result.succeeded, 'data': {
            'envelopeFile': str(Path(args.output).resolve()), 'stage': result.stage,
            'bodyFile': str(Path(args.body_output).resolve()) if args.body_output else None,
            'businessSuccess': result.business_success, 'contractMatched': result.contract_matched,
            'errorCode': result.error_code}}, ensure_ascii=False))
        return 0 if result.succeeded else 2
    except (OSError, ValueError, UnicodeError):
        print(json.dumps({'ok': False, 'error': {'code': 'RESULT_PARSE_FAILED',
            'message': '结果解析失败，请核对完整JSON或JSONL、最终信封及新的输出路径。'}}, ensure_ascii=False))
        return 2


if __name__ == '__main__':
    sys.exit(main())
