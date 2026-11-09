"""从实测日志生成可复制的中文测试报告；不重新调用业务接口。"""

import json
import os
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import List

from cli_result import load_json, read_text, write_new
from test_log import EndMark, append, iso, read_info, read_records


@dataclass
class SkillVersion:
    # Agent实际读取的技能名称。
    name: str
    # Agent实际读取入口所声明的版本，不推断客户端安装状态。
    version: str


@dataclass
class TestSummary:
    # 完成、部分完成、失败等实际结果。
    outcome: str
    # 实际读取的技能版本清单。
    skills: List[SkillVersion]
    # 离线version取得的CLI版本；未取得写未记录。
    cli_version: str
    # 本次可证明的数据和业务范围。
    coverage: List[str]
    # 未取得或不足以作结论的证据。
    limitations: List[str]
    # 本次实测问题；无问题可为空数组。
    issues: List[str]
    # 已复用的准备资料；不含凭证内容。
    reused: List[str]
    # 本次未执行的可选分支。
    not_performed: List[str]


def string_list(value, name):
    result = value.get(name, [])
    if not isinstance(result, list) or any(not isinstance(item, str) for item in result):
        raise ValueError('报告摘要的' + name + '必须是文字数组。')
    return result


def read_summary(path):
    value = load_json(read_text(path, 262144))
    if not isinstance(value, dict):
        raise ValueError('报告摘要须为具名对象。')
    skills = value.get('skills', [])
    if not isinstance(skills, list) or any(not isinstance(s, dict) or
            not isinstance(s.get('name'), str) or not isinstance(s.get('version'), str) for s in skills):
        raise ValueError('技能版本清单格式错误。')
    if any(not isinstance(value.get(k), str) for k in ('outcome', 'cliVersion')):
        raise ValueError('报告缺少结果或CLI版本说明。')
    return TestSummary(value['outcome'], [SkillVersion(s['name'], s['version']) for s in skills],
        value['cliVersion'], string_list(value, 'coverage'), string_list(value, 'limitations'),
        string_list(value, 'issues'), string_list(value, 'reused'), string_list(value, 'notPerformed'))


def cell(text):
    return str(text).replace('|', '\\|').replace('\n', ' ')


def seconds(start, end):
    return f'{(end - start) / 1000000000:.3f}' if end >= start else '未记录（墙钟逆跳）'


def clock(wall_ns, started_ns):
    value = iso(wall_ns)
    return value[11:23] if value[:10] == iso(started_ns)[:10] else value


def coverage_ns(calls, start, end):
    intervals = sorted((max(start, c.started_ns), min(end, c.ended_ns)) for c in calls
                       if c.ended_ns is not None and c.ended_ns >= c.started_ns
                       and c.started_ns < end and c.ended_ns > start)
    total, right = 0, start
    for left, stop in intervals:
        total += max(0, stop - max(left, right))
        right = max(right, stop)
    return total


def group_status(calls):
    groups = {}
    for c in calls:
        if c.group:
            groups.setdefault(c.group, []).append(c)
    notes = []
    for name, entries in groups.items():
        intervals = sorted((c.started_ns, c.ended_ns) for c in entries if c.ended_ns is not None)
        overlap = any(b[0] < a[1] for i, a in enumerate(intervals) for b in intervals[i + 1:])
        if len(entries) == 1:
            status = '仅一次调用'
        elif any(c.ended_ns is None for c in entries):
            status = '证据不完整'
        else:
            status = '存在实测重叠' if overlap else '实际串行，无时间重叠'
        notes.append(name + '：' + status)
    return '；'.join(notes) or '本轮未设置并行组'


def bullets(values, empty='无'):
    return '\n'.join('- ' + item for item in values) if values else '- ' + empty


def render(root, info, summary, business, calls, marks, ended_ns):
    versions = '；'.join(s.name + ' ' + s.version for s in summary.skills) or '未记录'
    lines = [f'# 【{info.title}】测试报告', '',
        '- 测试环境：' + info.environment, '- 业务条件：' + info.conditions,
        '- 实际读取技能版本：' + versions, '- CLI版本：' + summary.cli_version,
        '- 执行结果：' + summary.outcome, '- 开始时间：' + iso(info.started_ns),
        '- 结束时间：' + iso(ended_ns), '- 总墙钟耗时：' + seconds(info.started_ns, ended_ns) + ' 秒',
        '- 计时边界：首个start命令取时至首次完整报告保存完成；最终时间字段回填与聊天展示不计入。',
        '- 单次命令毫秒数由同进程单调时钟实测；总耗时和阶段由真实墙钟时间计算，未用接口耗时相加替代。',
        '', '## 业务结果', '', business.strip(), '', '## 阶段耗时', '',
        '| 阶段 | 开始 | 结束 | 墙钟秒数 | 计时命令覆盖秒数 | 未被命令覆盖秒数 |',
        '| --- | --- | --- | ---: | ---: | ---: |']
    for index, mark in enumerate(marks):
        end = marks[index + 1].at_ns if index + 1 < len(marks) else ended_ns
        covered = coverage_ns(calls, mark.at_ns, end)
        gap = f'{(end - mark.at_ns - covered) / 1000000000:.3f}' if end >= mark.at_ns else '未记录'
        lines.append('| ' + cell(mark.name) + ' | ' + clock(mark.at_ns, info.started_ns) + ' | ' + clock(end, info.started_ns) + ' | '
                     + seconds(mark.at_ns, end) + f' | {covered / 1000000000:.3f} | {gap} |')
    lines += ['', '未被命令覆盖时间包含模型思考、工具调度、参考读取和未包装的本地操作，不能全称为接口耗时或模型耗时。',
        '', '## 逐次调用日志', '',
        '| 序号 | 阶段／计划组 | 动作 | 开始 | 结束 | 毫秒 | 结果 | 尝试／再次执行原因 |',
        '| --- | --- | --- | --- | --- | ---: | --- | --- |']
    for index, c in enumerate(calls, 1):
        result = c.result + (f'（{c.error_code}）' if c.error_code else '')
        retry = str(c.attempt) + '／' + (c.retry_reason or '无')
        row = [index, c.stage + '／' + (c.group or '-'), c.name, clock(c.started_ns, info.started_ns),
               clock(c.ended_ns, info.started_ns) if c.ended_ns is not None else '未记录',
               f'{c.duration_ms:.3f}' if c.duration_ms is not None else '未记录', result, retry]
        lines.append('| ' + ' | '.join(cell(item) for item in row) + ' |')
    count = {kind: sum(c.kind == kind for c in calls) for kind in ('local', 'profile', 'resolve', 'view', 'read', 'write')}
    lines += ['', '计划组与实际并发：' + group_status(calls), '', '## 次数统计', '',
        f'- 账号元数据读取：{count["profile"]}；接口解析：{count["resolve"]}；视角查询：{count["view"]}。',
        f'- 业务读取尝试：{count["read"]}；其中成功：{sum(c.kind == "read" and c.result == "成功" for c in calls)}。',
        f'- 写入流程调用：{count["write"]}；本地命令：{count["local"]}。写入流程次数不等于实际发送次数。',
        f'- 再次执行：{sum(c.attempt > 1 for c in calls)}；失败：{sum(c.result == "失败" for c in calls)}；未结束：{sum(c.ended_ns is None for c in calls)}。',
        '', '## 覆盖与限制', '', bullets(summary.coverage, '未记录覆盖范围'), '',
        '证据不足：', '', bullets(summary.limitations), '', '本次未执行：', '', bullets(summary.not_performed),
        '', '## 实测问题与复用资料', '', bullets(summary.issues, '本轮未报告新增问题'), '',
        '已复用：', '', bullets(summary.reused, '未记录'), '', '## 文件位置', '',
        '- 耗时日志：' + str(root / 'timing.jsonl'), '- 测试报告：' + str(root / 'test-report.md'),
        '- 原始命令输出与标准错误：本任务目录各调用文件。', '']
    return '\n'.join(lines)


def finish_report(root, summary_file, business_file):
    info = read_info(root)
    if (root / 'test-finished.json').exists() or (root / 'test-report.md').exists():
        raise ValueError('报告已存在，不能重新计时或覆盖原测试。')
    summary = read_summary(summary_file)
    business = read_text(business_file, 262144)
    if not business.strip():
        raise ValueError('业务结果为空。')
    calls, marks = read_records(root)
    if any(c.ended_ns is None for c in calls):
        raise ValueError('还有调用未结束；先核对原调用，不重发业务请求。')
    report = root / 'test-report.md'
    write_new(report, render(root, info, summary, business, calls, marks, time.time_ns()))
    ended_ns = time.time_ns()
    # 固定首次完整保存的实测终点，再回填最终时间表；不估算保存耗时。
    final = render(root, info, summary, business, calls, marks, ended_ns)
    with tempfile.NamedTemporaryFile(dir=root, mode='w', encoding='utf-8', newline='\n',
                                     prefix='.report-', delete=False) as stream:
        stream.write(final)
        temporary = stream.name
    os.replace(temporary, report)
    append(root, EndMark(ended_ns))
    write_new(root / 'test-finished.json', json.dumps({'ended_ns': ended_ns,
        'ended_at': iso(ended_ns), 'report': str(report)}, ensure_ascii=False))
    return report, ended_ns
