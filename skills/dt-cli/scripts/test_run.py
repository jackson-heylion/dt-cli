"""复用测试计时、独立读取并发和报告生成；不自动安装、登录或重试。"""

import argparse
import json
import subprocess
import sys
import time
import uuid
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path
from typing import List, Optional

from cli_result import load_json, parse_cli_output, read_text, write_new
from test_log import (KINDS, CallLog, append, change_stage, create_run,
                      current_stage, new_target, read_records)
from test_report import finish_report


@dataclass
class CallSpec:
    # 本次调用的中文动作说明。
    name: str
    # 统计类别，不作为业务权限或执行许可。
    kind: str
    # 传给本地进程的参数数组；不使用shell，不写入计时日志。
    argv: List[str]
    # 是否按CLI最终信封核对结果。
    cli: bool
    # 本任务目录内的新标准输出文件名。
    output: str
    # 此动作的尝试序号，首次为1。
    attempt: int = 1
    # 再次执行原因；再次执行时必须说明。
    retry_reason: Optional[str] = None


@dataclass
class CallOutcome:
    # 原进程退出码或信封核对失败的非零码。
    return_code: int
    # 该次调用实际计时记录。
    log: CallLog
    # 解析成功后的新信封文件，缺失时为空。
    envelope_file: Optional[str]
    # 解析成功的原业务正文文件，接口无正文时为空。
    body_file: Optional[str]


def validate_spec(spec):
    if not isinstance(spec.name, str) or not spec.name or spec.kind not in KINDS or not isinstance(spec.argv, list) or not spec.argv or any(
            not isinstance(arg, str) or not arg for arg in spec.argv):
        raise ValueError('调用须有动作名、统计类别和完整参数数组。')
    if type(spec.cli) is not bool or type(spec.attempt) is not int or spec.attempt < 1:
        raise ValueError('CLI标记或尝试序号无效。')
    if spec.attempt > 1 and not spec.retry_reason:
        raise ValueError('再次执行须记录原因，工具不会自动重试。')
    if spec.retry_reason is not None and not isinstance(spec.retry_reason, str):
        raise ValueError('再次执行原因须为中文说明。')


def output_paths(root, spec):
    output = new_target(root, spec.output)
    error = new_target(root, spec.output + '.stderr.log')
    envelope = new_target(root, spec.output + '.envelope.json')
    body = new_target(root, spec.output + '.body.json')
    return output, error, envelope, body


def execute(root, spec, stage, group):
    validate_spec(spec)
    output, error, envelope, body = output_paths(root, spec)
    started_ns, tick = time.time_ns(), time.perf_counter_ns()
    record = CallLog(str(uuid.uuid4()), spec.name, stage, spec.kind, group, started_ns,
                     None, None, None, '执行中', None, spec.attempt, spec.retry_reason, str(output))
    append(root, record)
    envelope_file, body_file, return_code = None, None, 2
    try:
        with output.open('xb') as stdout, error.open('xb') as stderr:
            process = subprocess.run(spec.argv, cwd=root, stdout=stdout, stderr=stderr, check=False)
        record.exit_code = process.returncode
        return_code = process.returncode
        record.result = '成功' if process.returncode == 0 else '失败'
        if spec.cli:
            response = parse_cli_output(read_text(output))
            write_new(envelope, response.envelope_json)
            envelope_file = str(envelope)
            if response.body_json is not None:
                write_new(body, response.body_json)
                body_file = str(body)
            if not response.succeeded:
                record.result = '失败'
                return_code = process.returncode or 2
                record.error_code = response.error_code or 'BUSINESS_OR_CONTRACT_FAILED'
    except KeyboardInterrupt:
        record.result, record.error_code = '结果未知', 'LOCAL_INTERRUPTED'
        return_code = 130
    except (OSError, ValueError, UnicodeError):
        record.result, record.error_code = '失败', 'LOCAL_EXECUTION_OR_OUTPUT_FAILED'
        return_code = return_code or 2
    finally:
        record.ended_ns = time.time_ns()
        record.duration_ms = (time.perf_counter_ns() - tick) / 1000000
        append(root, record)
    return CallOutcome(return_code, record, envelope_file, body_file)


def result_wire(outcome):
    return {'name': outcome.log.name, 'outputFile': outcome.log.output_file,
            'envelopeFile': outcome.envelope_file, 'bodyFile': outcome.body_file,
            'durationMs': outcome.log.duration_ms, 'exitCode': outcome.log.exit_code,
            'result': outcome.log.result, 'errorCode': outcome.log.error_code}


def read_plan(path):
    value = load_json(read_text(path, 262144))
    if not isinstance(value, dict) or not isinstance(value.get('calls'), list):
        raise ValueError('并发计划须为具名calls数组。')
    width = value.get('parallelism', 3)
    if type(width) is not int or not 1 <= width <= 4 or not 1 <= len(value['calls']) <= 20:
        raise ValueError('并发数须为1至4，单组调用数须为1至20。')
    specs = []
    for item in value['calls']:
        if not isinstance(item, dict):
            raise ValueError('调用计划须为具名对象。')
        spec = CallSpec(item.get('name'), item.get('kind'), item.get('argv'),
                        item.get('cli', True), item.get('output'), item.get('attempt', 1),
                        item.get('retryReason'))
        validate_spec(spec)
        if not isinstance(spec.output, str) or not spec.output:
            raise ValueError('调用缺少新的输出文件名。')
        if spec.kind == 'write' or 'writes' in spec.argv:
            raise ValueError('并发计划只用于独立读取或本地准备；写入流程按原步骤顺序执行。')
        specs.append(spec)
    return width, specs


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    start = sub.add_parser('start', help='首次执行前创建新的任务和起点')
    start.add_argument('--directory', required=True)
    start.add_argument('--title', required=True)
    start.add_argument('--environment', required=True)
    start.add_argument('--conditions', required=True)
    stage = sub.add_parser('stage', help='切换阶段；保留阶段内的全部墙钟时间')
    stage.add_argument('--directory', required=True)
    stage.add_argument('--name', required=True)
    run = sub.add_parser('run', help='计时执行一次本地命令，不自动重试')
    run.add_argument('--directory', required=True)
    run.add_argument('--name', required=True)
    run.add_argument('--kind', choices=KINDS, required=True)
    run.add_argument('--output', required=True)
    run.add_argument('--cli', action='store_true')
    run.add_argument('--group')
    run.add_argument('--attempt', type=int, default=1)
    run.add_argument('--retry-reason')
    run.add_argument('command', nargs=argparse.REMAINDER)
    batch = sub.add_parser('batch', help='并发执行一组已确认独立的读取')
    batch.add_argument('--directory', required=True)
    batch.add_argument('--plan', required=True)
    batch.add_argument('--group', required=True)
    finish = sub.add_parser('finish', help='从日志和一次业务结果生成中文报告')
    finish.add_argument('--directory', required=True)
    finish.add_argument('--summary', required=True)
    finish.add_argument('--business', required=True)
    args = parser.parse_args(argv)
    try:
        root = Path(args.directory).resolve()
        if args.action == 'start':
            root = create_run(root, args.title, args.environment, args.conditions)
            data = {'directory': str(root), 'timingFile': str(root / 'timing.jsonl')}
            code = 0
        elif args.action == 'stage':
            change_stage(root, args.name)
            data, code = {'stage': args.name}, 0
        elif args.action == 'finish':
            report, end = finish_report(root, args.summary, args.business)
            data, code = {'reportFile': str(report), 'timingFile': str(root / 'timing.jsonl'),
                          'endedNs': end}, 0
        elif args.action == 'run':
            command = args.command[1:] if args.command[:1] == ['--'] else args.command
            spec = CallSpec(args.name, args.kind, command, args.cli, args.output,
                            args.attempt, args.retry_reason)
            outcome = execute(root, spec, current_stage(root), args.group)
            data, code = result_wire(outcome), outcome.return_code
        else:
            phase = current_stage(root)
            width, specs = read_plan(args.plan)
            targets = []
            for spec in specs:
                targets.extend(output_paths(root, spec))
            if len(set(targets)) != len(targets):
                raise ValueError('并发调用的输出文件重叠，尚未执行。')
            with ThreadPoolExecutor(max_workers=width) as pool:
                futures = [pool.submit(execute, root, spec, phase, args.group) for spec in specs]
                outcomes = [future.result() for future in futures]
            code = next((o.return_code for o in outcomes if o.return_code != 0), 0)
            data = {'calls': [result_wire(o) for o in outcomes]}
        print(json.dumps({'ok': code == 0, 'data': data}, ensure_ascii=False))
        return code if 0 <= code <= 255 else 2
    except (OSError, ValueError, UnicodeError, TypeError, KeyError):
        print(json.dumps({'ok': False, 'error': {'code': 'TEST_TOOL_FAILED',
            'message': '测试工具未完成。请核对新任务目录、参数数组、输入和输出文件；保留原调用，不自动重发。'}},
            ensure_ascii=False))
        return 2


if __name__ == '__main__':
    sys.exit(main())
