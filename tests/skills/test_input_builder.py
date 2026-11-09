"""验证技能构造的输入符合CLI线格式，保留值及隐私边界。"""

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

BUILDER = pathlib.Path(__file__).resolve().parents[2] / 'skills/dt-cli/scripts/build_input.py'


class InputBuilderTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)

    def write(self, name, text):
        path = self.root / name
        path.write_text(text, encoding='utf-8')
        return path

    def run_builder(self, *arguments):
        return subprocess.run([sys.executable, str(BUILDER), *map(str, arguments)],
                              capture_output=True, text=True, timeout=10)

    def test_get_retains_scalar_types_arrays_and_full_view(self):
        query = self.write('query.json', '{"code":"00123","pageNum":1,"enable":false,"ids":[9007199254740993,2]}')
        output = self.root / 'input.json'
        result = self.run_builder('--query-file', query, '--view-id', '9',
                                  '--object-id', '01', '--output', output)
        self.assertEqual(result.returncode, 0, result.stdout)
        wire = json.loads(output.read_text())
        parameters = {item['name']: item['value'] for item in wire['query']}
        self.assertEqual(parameters['code'], '00123')
        self.assertIs(type(parameters['pageNum']), int)
        self.assertIs(parameters['enable'], False)
        self.assertEqual(parameters['ids'], [9007199254740993, 2])
        self.assertEqual(wire['view'], {'viewId': 9, 'objectId': '01'})
        self.assertEqual(json.loads(result.stdout)['data']['arguments'],
                         ['--input-file', str(output.resolve())])

    def test_post_body_round_trips_without_losing_precision_or_text(self):
        raw = '{"name":"中文\\\"引号","amount":0.123456789123456789,"enabled":false}'
        body = self.write('body.json', raw)
        output = self.root / 'input.json'
        result = self.run_builder('--body-file', body, '--output', output)
        self.assertEqual(result.returncode, 0, result.stdout)
        wire = json.loads(output.read_text())
        self.assertEqual(wire['bodyJson'], raw)
        self.assertIsNone(wire['view'])
        self.assertEqual(wire['query'], [])
        self.assertNotIn('amount', wire)

    def test_invalid_transport_values_do_not_create_output_or_echo_data(self):
        for raw in ['{"name":null}', '{"ids":[1,null]}', '{"name":{"secret":"私密值"}}',
                    '{"pageNum":1,"pageNum":2}', '{"value":NaN}']:
            with self.subTest(raw=raw):
                query = self.write('query.json', raw)
                output = self.root / 'input.json'
                result = self.run_builder('--query-file', query, '--output', output)
                self.assertEqual(result.returncode, 2)
                self.assertFalse(output.exists())
                self.assertNotIn('私密值', result.stdout + result.stderr)

    def test_body_duplicates_and_incomplete_views_are_rejected(self):
        body = self.write('body.json', '{"scope":"one","scope":"two"}')
        output = self.root / 'input.json'
        for args in [('--body-file', body), ('--view-id', '9'),
                     ('--object-id', '1'), ('--view-id', '0', '--object-id', '1'),
                     ('--view-id', '4294967296', '--object-id', '1')]:
            with self.subTest(args=args):
                result = self.run_builder(*args, '--output', output)
                self.assertEqual(result.returncode, 2)
                self.assertFalse(output.exists())

    def test_existing_output_is_preserved(self):
        output = self.write('input.json', '已有内容')
        result = self.run_builder('--output', output)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(json.loads(result.stdout)['error']['code'], 'OUTPUT_EXISTS')
        self.assertEqual(output.read_text(), '已有内容')

    def contract(self):
        value = {'ok': True, 'data': {'contract': {'operationId': 'op_test',
            'operationVersion': 1, 'requestBodyRequired': False, 'parameters': [
                {'name': 'id', 'location': 'PATH', 'required': True},
                {'name': 'pageNum', 'location': 'QUERY', 'required': True},
                {'name': 'pageSize', 'location': 'QUERY', 'required': True}]}}}
        return self.write('resolve.json', json.dumps(value))

    def test_misplaced_path_id_is_rejected_before_any_output_is_created(self):
        query = self.write('query.json', '{"id":32,"pageNum":1,"pageSize":100}')
        output = self.root / 'input.json'
        result = self.run_builder('--contract-file', self.contract(), '--query-file', query, '--output', output)
        self.assertEqual(result.returncode, 2)
        error = json.loads(result.stdout)['error']
        self.assertEqual(error['code'], 'PARAMETER_LOCATION')
        self.assertIn('PATH', error['message'])
        self.assertIn('QUERY', error['message'])
        self.assertNotIn('32', result.stdout)
        self.assertFalse(output.exists())

    def test_correct_path_and_query_are_generated_from_fixed_contract(self):
        path = self.write('path.json', '{"id":32}')
        query = self.write('query.json', '{"pageNum":1,"pageSize":100}')
        output = self.root / 'input.json'
        result = self.run_builder('--contract-file', self.contract(), '--path-file', path,
                                  '--query-file', query, '--output', output)
        self.assertEqual(result.returncode, 0, result.stdout)
        wire = json.loads(output.read_text())
        self.assertEqual(wire['path'], [{'name': 'id', 'value': 32}])
        self.assertNotIn('id', {p['name'] for p in wire['query']})
        self.assertEqual(json.loads(result.stdout)['data']['operationId'], 'op_test')

    def test_unknown_or_missing_parameters_stop_offline(self):
        path = self.write('path.json', '{"id":32}')
        for raw, code in [('{"pageNum":1}', 'REQUIRED_PARAMETER'),
                          ('{"pageNum":1,"pageSize":5,"invented":true}', 'UNKNOWN_PARAMETER')]:
            query = self.write('query.json', raw)
            output = self.root / 'input.json'
            result = self.run_builder('--contract-file', self.contract(), '--path-file', path,
                                      '--query-file', query, '--output', output)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(json.loads(result.stdout)['error']['code'], code)
            self.assertFalse(output.exists())

    def test_windows_utf8_bom_is_accepted_without_transmitting_the_bom(self):
        body = self.write('body.json', '\ufeff{"name":"中文"}')
        output = self.root / 'input.json'
        result = self.run_builder('--body-file', body, '--output', output)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(json.loads(output.read_text())['bodyJson'], '{"name":"中文"}')
        self.assertFalse(output.read_bytes().startswith(b'\xef\xbb\xbf'))

    def test_body_size_is_bounded_before_output(self):
        body = self.write('body.json', json.dumps({'name': 'x' * 65536}))
        output = self.root / 'input.json'
        result = self.run_builder('--body-file', body, '--output', output)
        self.assertEqual(result.returncode, 2)
        self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
