"""测试计时的具名记录和本地日志；日志不保存命令参数或凭证。"""

import json
import os
import time
from contextlib import contextmanager
from dataclasses import asdict, dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import List, Optional

from cli_result import load_json, read_text, write_new

KINDS = ('local', 'profile', 'resolve', 'view', 'read', 'write')
ZONE = timezone(timedelta(hours=8))


@dataclass
class RunInfo:
    # 用户本次测试名称。
    title: str
    # 本次环境；只作测试说明，不改变CLI账号。
    environment: str
    # 已固定的业务条件，不含凭证。
    conditions: str
    # 首个计时命令实际取到的Unix墙钟纳秒。
    started_ns: int


@dataclass
class StageMark:
    # 阶段从此打点开始；与下一个阶段衔接。
    name: str
    # 实际Unix墙钟纳秒，不使用单调时钟换算日期。
    at_ns: int


@dataclass
class EndMark:
    # 首次完整测试报告保存完成的实测墙钟纳秒。
    ended_ns: int


@dataclass
class CallLog:
    # 一次本地进程调用的唯一编号；开始与结束记录共用。
    call_id: str
    # 调用的中文动作说明，不保存原始命令行。
    name: str
    # 调用开始时所处的阶段。
    stage: str
    # local/profile/resolve/view/read/write，用于分开统计。
    kind: str
    # 计划并行组；是否实际重叠另由时间区间判断。
    group: Optional[str]
    # 实际进程开始墙钟纳秒。
    started_ns: int
    # 实际结束墙钟纳秒；中断且未结束时为空。
    ended_ns: Optional[int]
    # 同一进程用单调时钟计算的毫秒耗时；未结束时为空。
    duration_ms: Optional[float]
    # 原进程退出码；未结束或未启动时为空。
    exit_code: Optional[int]
    # 执行中、成功、失败或结果未知。
    result: str
    # 受控技术错误码；没有错误时为空。
    error_code: Optional[str]
    # 本动作的尝试序号；首次为1。
    attempt: int
    # 再次执行的已说明原因；首次为空。
    retry_reason: Optional[str]
    # 原始标准输出文件；只保存文件路径。
    output_file: str


def iso(wall_ns):
    return datetime.fromtimestamp(wall_ns / 1000000000, ZONE).isoformat(timespec='milliseconds')


@contextmanager
def log_lock(root):
    # 同时运行的计时进程只在短暂日志写入期间互斥。
    with (root / '.timing.lock').open('a+b') as stream:
        if os.name == 'nt':
            import msvcrt
            stream.seek(0)
            if not stream.read(1):
                stream.write(b'0')
                stream.flush()
            deadline = time.monotonic() + 5
            while True:
                try:
                    stream.seek(0)
                    msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
                    break
                except OSError:
                    if time.monotonic() >= deadline:
                        raise ValueError('计时日志正忙，请保留原任务。')
                    time.sleep(0.02)
            try:
                yield
            finally:
                stream.seek(0)
                msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)
        else:
            import fcntl
            fcntl.flock(stream.fileno(), fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(stream.fileno(), fcntl.LOCK_UN)


def append(root, record):
    wire = asdict(record)
    if isinstance(record, CallLog):
        wire.update(entry_type='call', started_at=iso(record.started_ns),
                    ended_at=iso(record.ended_ns) if record.ended_ns is not None else None)
    elif isinstance(record, StageMark):
        wire.update(entry_type='stage', at=iso(record.at_ns))
    else:
        wire.update(entry_type='end', ended_at=iso(record.ended_ns))
    line = (json.dumps(wire, ensure_ascii=False, allow_nan=False) + '\n').encode('utf-8')
    with log_lock(root):
        descriptor = os.open(root / 'timing.jsonl', os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
        try:
            if os.write(descriptor, line) != len(line):
                raise OSError('日志写入不完整。')
        finally:
            os.close(descriptor)


def create_run(directory, title, environment, conditions):
    started_ns = time.time_ns()
    root = Path(directory).resolve()
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    info = RunInfo(title, environment, conditions, started_ns)
    write_new(root / 'test-state.json', json.dumps(asdict(info), ensure_ascii=False, indent=2))
    append(root, StageMark('程序准备', started_ns))
    return root


def read_info(root):
    value = load_json(read_text(root / 'test-state.json', 65536))
    if not isinstance(value, dict) or any(not isinstance(value.get(k), str)
            for k in ('title', 'environment', 'conditions')) or type(value.get('started_ns')) is not int:
        raise ValueError('测试状态格式不正确。')
    return RunInfo(value['title'], value['environment'], value['conditions'], value['started_ns'])


def read_records(root):
    calls = {}
    marks = []
    for line in read_text(root / 'timing.jsonl').splitlines():
        if not line.strip():
            continue
        value = load_json(line)
        if not isinstance(value, dict):
            raise ValueError('计时日志格式不正确。')
        if value.get('entry_type') == 'call':
            record = CallLog(**{key: value[key] for key in CallLog.__dataclass_fields__})
            calls[record.call_id] = record
        elif value.get('entry_type') == 'stage':
            marks.append(StageMark(value['name'], value['at_ns']))
        elif value.get('entry_type') == 'end':
            continue
        else:
            raise ValueError('未知计时记录类型。')
    return sorted(calls.values(), key=lambda c: c.started_ns), marks


def current_stage(root):
    read_info(root)
    if (root / 'test-finished.json').exists():
        raise ValueError('本次测试已结束，请使用新的任务目录。')
    _, marks = read_records(root)
    return marks[-1].name if marks else '程序准备'


def change_stage(root, name):
    current_stage(root)
    calls, _ = read_records(root)
    if any(c.ended_ns is None for c in calls):
        raise ValueError('仍有调用执行中；结束后再切换阶段。')
    append(root, StageMark(name, time.time_ns()))


def new_target(root, file_name):
    target = (root / file_name).resolve()
    try:
        target.relative_to(root)
    except ValueError:
        raise ValueError('输出文件须位于本次任务目录。')
    if target.exists():
        raise ValueError('输出已存在，请为新尝试选择新文件。')
    target.parent.mkdir(parents=True, exist_ok=True)
    return target
