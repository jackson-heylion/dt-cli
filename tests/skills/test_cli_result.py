"""验证事件输出、缺省stage与业务成功标记，不涉及真实写入。"""

import importlib.util
import json
import pathlib
import sys
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[2] / 'skills/dt-cli/scripts/cli_result.py'
SPEC = importlib.util.spec_from_file_location('dt_cli_result_tested', SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class CliResultTest(unittest.TestCase):
    def test_write_event_and_final_envelope_are_not_parsed_as_one_json(self):
        event = '{"event":"platform.writes","stage":"prepared"}'
        final = '{"ok":true,"data":{"stage":"prepared","recordId":"sample"}}'
        result = MODULE.parse_cli_output(event + '\n' + final + '\n')
        self.assertTrue(result.succeeded)
        self.assertEqual(result.stage, 'prepared')
        self.assertEqual(result.envelope_json, final)

    def test_inspect_without_stage_is_valid_and_keeps_fingerprint(self):
        result = MODULE.parse_cli_output('{"ok":true,"data":{"fingerprint":"原指纹"}}')
        self.assertTrue(result.succeeded)
        self.assertIsNone(result.stage)
        self.assertEqual(json.loads(result.envelope_json)['data']['fingerprint'], '原指纹')

    def test_body_string_precision_is_retained_and_business_failure_is_not_success(self):
        body = '{"price":0.123456789123456789,"name":"中文"}'
        raw = json.dumps({'ok': True, 'data': {'body': body, 'businessSuccess': False,
                                              'contractMatched': True}})
        result = MODULE.parse_cli_output(raw)
        self.assertFalse(result.succeeded)
        self.assertEqual(result.body_json, body)

    def test_event_only_malformed_output_and_false_contract_cannot_be_success(self):
        for raw in ('{"event":"prepared"}', '{"ok":true,"data":{}}\nnot-json',
                    '{"ok":true,"ok":false,"data":{}}'):
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                MODULE.parse_cli_output(raw)
        result = MODULE.parse_cli_output('{"ok":true,"data":{"contractMatched":false}}')
        self.assertFalse(result.succeeded)


if __name__ == '__main__':
    unittest.main()
