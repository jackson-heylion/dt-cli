"""验证真实计时、实际并发、一次执行及自动报告；只运行离线假CLI。"""

import json
import pathlib
import subprocess
import sys
import tempfile
import time
import unittest
from datetime import datetime

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / 'skills/dt-cli/scripts/test_run.py'


class TestRunTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.folder = pathlib.Path(self.temp.name)
        self.task = self.folder / 'task'
        self.fake = self.folder / 'fake.py'
        self.fake.write_text('import json, sys, time\n'
            'time.sleep(0.08)\n'
            'print(json.dumps({"event":"sample","stage":"prepared"}))\n'
            'print(json.dumps({"ok":True,"data":{"businessSuccess":True,"contractMatched":True,\n'
            '"body":"{\\"list\\":[],\\"total\\":0}"}}))\n', encoding='utf-8')
        result = self.invoke('start', '--title', '离线样本', '--environment', '离线测试', '--conditions', '不调用网络')
        self.assertEqual(result.returncode, 0, result.stdout)

    def invoke(self, action, *args):
        return subprocess.run([sys.executable, str(SCRIPT), action, '--directory', str(self.task),
                               *map(str, args)], capture_output=True, text=True, timeout=10)

    def records(self):
        records = [json.loads(line) for line in (self.task / 'timing.jsonl').read_text().splitlines()]
        calls = {}
        for record in records:
            if record['entry_type'] == 'call':
                calls[record['call_id']] = record
        return records, list(calls.values())

    def finish(self):
        summary = self.folder / 'summary.json'
        summary.write_text(json.dumps({'outcome': '完成', 'skills': [{'name': 'dt-cli', 'version': '测试'}],
            'cliVersion': '离线假CLI', 'coverage': ['仅离线样本'], 'limitations': [],
            'issues': [], 'reused': [], 'notPerformed': ['未发送业务消息']}, ensure_ascii=False))
        business = self.folder / 'business.md'
        business.write_text('离线样本完成，未查询业务数据。', encoding='utf-8')
        result = self.invoke('finish', '--summary', summary, '--business', business)
        self.assertEqual(result.returncode, 0, result.stdout)
        return (self.task / 'test-report.md').read_text()

    def test_call_logs_real_wall_time_and_monotonic_duration_without_command_arguments(self):
        result = self.invoke('run', '--name', '读取样本', '--kind', 'read', '--cli', '--output', 'call.json',
                             '--', sys.executable, self.fake, '凭证占位不可出现在报告')
        self.assertEqual(result.returncode, 0, result.stdout)
        _, calls = self.records()
        self.assertEqual(len(calls), 1)
        record = calls[0]
        self.assertGreater(record['duration_ms'], 70)
        self.assertGreater(record['ended_ns'], record['started_ns'])
        self.assertGreater(datetime.fromisoformat(record['started_at']).year, 2000)
        self.assertEqual(datetime.fromisoformat(record['started_at']).utcoffset().total_seconds(), 28800)
        self.assertEqual(json.loads((self.task / 'call.json.body.json').read_text()), {'list': [], 'total': 0})
        report = self.finish()
        self.assertNotIn('凭证占位不可出现在报告', report)
        self.assertNotIn('凭证占位不可出现在报告', (self.task / 'timing.jsonl').read_text())
        all_records, _ = self.records()
        self.assertEqual(all_records[-1]['entry_type'], 'end')
        self.assertLessEqual(all_records[-1]['ended_ns'], time.time_ns())

    def test_batch_really_overlaps_and_report_does_not_sum_durations_as_total(self):
        plan = self.folder / 'plan.json'
        plan.write_text(json.dumps({'parallelism': 2, 'calls': [
            {'name': name, 'kind': 'read', 'cli': True, 'output': name + '.json',
             'argv': [sys.executable, str(self.fake)]} for name in ('指标', '营业额')]}))
        result = self.invoke('batch', '--plan', plan, '--group', '主数据')
        self.assertEqual(result.returncode, 0, result.stdout)
        _, calls = self.records()
        self.assertEqual(len(calls), 2)
        self.assertLess(max(c['started_ns'] for c in calls), min(c['ended_ns'] for c in calls))
        report = self.finish()
        self.assertIn('主数据：存在实测重叠', report)
        self.assertIn('未用接口耗时相加替代', report)

    def test_same_group_name_on_sequential_calls_is_reported_as_serial(self):
        for number in (1, 2):
            result = self.invoke('run', '--name', f'解析{number}', '--kind', 'resolve', '--cli',
                '--group', '准备', '--output', f'call{number}.json', '--', sys.executable, self.fake)
            self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn('准备：实际串行，无时间重叠', self.finish())

    def test_failure_is_logged_once_and_is_never_automatically_retried(self):
        failed = self.folder / 'fail.py'
        failed.write_text('import json, sys\nprint(json.dumps({"ok":False, "error":{"code":"REQUIRED_PARAMETER"}}))\nsys.exit(2)\n')
        result = self.invoke('run', '--name', '失败样本', '--kind', 'read', '--cli', '--output', 'failed.json',
                             '--', sys.executable, failed)
        self.assertEqual(result.returncode, 2)
        _, calls = self.records()
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0]['result'], '失败')
        self.assertEqual(calls[0]['error_code'], 'REQUIRED_PARAMETER')
        self.assertIsNotNone(calls[0]['ended_ns'])

    def test_existing_output_and_write_batch_are_rejected_before_execution(self):
        output = self.task / 'prior.json'
        output.write_text('原证据')
        result = self.invoke('run', '--name', '读取', '--kind', 'read', '--cli', '--output', 'prior.json',
                             '--', sys.executable, self.fake)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(output.read_text(), '原证据')
        plan = self.folder / 'write-plan.json'
        plan.write_text(json.dumps({'calls': [{'name': '发送', 'kind': 'write', 'cli': True,
            'output': 'write.json', 'argv': [sys.executable, str(self.fake)]}]}))
        result = self.invoke('batch', '--plan', plan, '--group', '禁止的发送组')
        self.assertEqual(result.returncode, 2)
        _, calls = self.records()
        self.assertEqual(calls, [])
        self.assertFalse((self.task / 'write.json').exists())

    def test_profile_read_is_not_counted_as_interface_resolution_and_report_cannot_be_retimed(self):
        for kind in ('profile', 'resolve'):
            result = self.invoke('run', '--name', kind, '--kind', kind, '--cli', '--output', kind + '.json',
                                 '--', sys.executable, self.fake)
            self.assertEqual(result.returncode, 0, result.stdout)
        report = self.finish()
        self.assertIn('账号元数据读取：1；接口解析：1', report)
        finished = (self.task / 'test-finished.json').read_bytes()
        result = self.invoke('stage', '--name', '重新开始')
        self.assertEqual(result.returncode, 2)
        self.assertEqual((self.task / 'test-finished.json').read_bytes(), finished)


if __name__ == '__main__':
    unittest.main()
