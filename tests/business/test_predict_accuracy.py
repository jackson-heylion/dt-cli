"""Confirmed business behavior, using explicit synthetic data and no remote calls."""
import copy
from datetime import date, timedelta
from decimal import Decimal
import hashlib
import importlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / 'skills/predict-accuracy-attribution/scripts'
sys.path.insert(0, str(SCRIPTS))
analysis = importlib.import_module('analyze')
runner = importlib.import_module('run')


def record(identity, accuracy='0.9', **overrides):
    return dict(recordId=identity, id=identity, shopCode='S1', orderDate='2026-10-05', arrivalDate='2026-10-06',
                orderPeriod=1, itemCode='104030007', orderUnit='kg', specification='1kg',
                predictOrderAccuracy=accuracy, manualOrderAccuracy=accuracy, predictConsumeAccuracy=accuracy,
                inventoryAccuracy=accuracy, suggestedPredictOrderNum='10', multipliedActualOrderNum='10',
                orderNum='10', predictConsumeNum='10', actualConsumeNum='10', theoryInventory='10',
                arrivalBeforeActualInventory='10', **overrides)


def bundle(records):
    return {'sourceKind': '构造数据，仅验证计算规则', 'context': {'subjectId': '2', 'environment': 'fixture', 'brandId': 1,
            'authorizedShopCodes': ['S1', 'S2'], 'shopCodes': ['S1', 'S2'], 'startDate': '2026-10-05', 'endDate': '2026-10-11'},
            'coverage': {'complete': True}, 'data': {'indicators': records, 'orders': copy.deepcopy(records),
            'items': [{'code': '104030007', 'specification': '1kg'}, {'code': 'ALT', 'specification': '1kg'}],
            'groups': [{'itemCode': '104030007', 'memberCodes': ['104030007'], 'complete': True}],
            'shops': [{'shopCode': 'S1', 'region': '华东'}, {'shopCode': 'S2', 'region': '华南'}]}}


class CalculationTest(unittest.TestCase):
    def test_calendar_dates_and_long_period_segments(self):
        start, end, _ = analysis.dates('上周', today='2026-10-13')
        self.assertEqual((str(start), str(end)), ('2026-10-05', '2026-10-11'))
        start, end, _ = analysis.dates('近7天', today='2026-10-13')
        self.assertEqual((str(start), str(end)), ('2026-10-06', '2026-10-12'))
        _, _, segments = analysis.dates(start='2026-08-01', end='2026-09-04', today='2026-10-13')
        self.assertEqual(segments, [{'startDate': '2026-08-01', 'endDate': '2026-08-31'}, {'startDate': '2026-09-01', 'endDate': '2026-09-04'}])

    def test_equal_weight_independent_denominators_and_id_deduplication(self):
        rows = [record('1', '0.9'), record('2', '0.7'), record('3', None)]
        rows[1]['manualOrderAccuracy'] = None
        rows.append(copy.deepcopy(rows[0]))
        result = analysis.analyze(bundle(rows), today='2026-10-13')
        order = result['metrics']['predictOrderAccuracy']
        manual = result['metrics']['manualOrderAccuracy']
        self.assertEqual((order['validCount'], order['excludedCount'], order['passShare']), (2, 1, Decimal('0.5')))
        self.assertEqual((manual['validCount'], manual['excludedCount']), (1, 2))
        self.assertEqual(result['quality']['indicators']['duplicates'], 1)
        self.assertEqual(result['rankMode'], '观察排序')
        self.assertIn('回写完成未核实', ' '.join(result['issues']))

    def test_different_real_ids_stay_and_changed_ids_are_excluded(self):
        rows = [record('one'), record('two')]
        self.assertEqual(analysis.analyze(bundle(rows), today='2026-10-13')['metrics']['predictOrderAccuracy']['validCount'], 2)
        changed = record('one', '0.2')
        result = analysis.analyze(bundle([*rows, changed]), today='2026-10-13')
        self.assertEqual(result['quality']['indicators']['changedIds'], ['one'])
        self.assertEqual(result['metrics']['predictOrderAccuracy']['validCount'], 1)

    def test_unknown_identity_cannot_enter_statistics(self):
        row = record(None)
        row.pop('id')
        result = analysis.analyze(bundle([row]), today='2026-10-13')
        self.assertEqual(result['metrics']['predictOrderAccuracy']['validCount'], 0)
        self.assertIn('缺少可靠原记录编号', result['metrics']['predictOrderAccuracy']['exclusions'])

    def test_group_inheritance_conflict_and_unknown_stay_separate(self):
        mapping = {'items': [{'itemCode': 'A', 'classification': '鲜鸡'}, {'itemCode': 'B', 'classification': '鲜猪'}]}
        items = [{'code': code} for code in ['A', 'B', 'ALT', 'UNKNOWN', 'NON']]
        groups = [{'itemCode': 'ALT', 'memberCodes': ['ALT', 'A'], 'complete': True},
                  {'itemCode': 'NON', 'memberCodes': ['NON'], 'complete': True}]
        result = analysis.classification(items, groups, [], mapping)
        self.assertEqual(result['ALT']['classification'], '鲜鸡')
        self.assertEqual(result['UNKNOWN']['classification'], '未知')
        self.assertEqual(result['NON']['classification'], '非鲜品')
        groups.append({'itemCode': 'ALT', 'memberCodes': ['ALT', 'B'], 'complete': True})
        self.assertEqual(analysis.classification(items, groups, [], mapping)['ALT']['classification'], '冲突')

    def test_cutoff_ties_and_overlapping_consumption(self):
        rows = [{'shopCode': str(i), 'passShare': Decimal(value)} for i, value in enumerate(['0.1', '0.2', '0.3', '0.3', '0.5'])]
        self.assertEqual(len(analysis.worst(rows, 3)), 4)
        orders = [record('1'), record('2')]
        for row in orders:
            row['itemCode'] = 'NON'
        result = analysis.top_consumption(orders, {'NON': {'classification': '非鲜品'}}, {'NON': {'specification': '1kg'}})
        self.assertFalse(result[0]['trusted'])
        self.assertEqual(result[0]['items'], [])

    def test_valid_revenue_days_and_store_specific_baselines(self):
        rows = []
        for shop, amounts in [('S1', ['10', '20', '18']), ('S2', ['20', '40', '36'])]:
            for when, amount in zip(['2026-10-01', '2026-10-03', '2026-10-05'], amounts):
                rows.append({'shopCode': shop, 'itemCode': 'A', 'statisticDate': when, 'actualRevenue': '10000', 'theoryConsume': amount, 'orderUnit': 'kg'})
            rows.append({'shopCode': shop, 'itemCode': 'A', 'statisticDate': '2026-10-02', 'actualRevenue': '0', 'theoryConsume': '999', 'orderUnit': 'kg'})
        result = analysis.daily_anomalies(rows, date(2026, 10, 5), date(2026, 10, 5))
        self.assertEqual([row['baseline'] for row in result], [Decimal('15'), Decimal('30')])
        self.assertTrue(all(row['sampleCount'] == 2 and row['deviation'] == Decimal('0.2') and row['anomaly'] for row in result))

    def test_fresh_rule_needs_item_date_and_history_evidence(self):
        row = record('1')
        wrong = {'historicalEffectiveVerified': True, 'fresh': True, 'itemCode': 'OTHER', 'effectiveFrom': '2026-01-01', 'evidenceSource': 'fixture'}
        self.assertEqual(analysis.fresh_inventory_rule(row, [wrong])['mode'], 'unverified')
        right = {**wrong, 'itemCode': row['itemCode']}
        self.assertEqual(analysis.fresh_inventory_rule(row, [right])['mode'], 'fresh')

    def test_inventory_duplicate_and_partial_warehouse_evidence(self):
        bills = [{'inventoryNo': 'bill-1', 'shopCode': 'S1', 'warehouseId': 1, 'inventoryDate': '2026-10-05', 'type': 'DAY_INVENTORY', 'state': 'FINISHED'}] * 2
        result = analysis.inventory_summary(bills, [], date(2026, 10, 5), date(2026, 10, 5))
        self.assertEqual(result['quality']['duplicates'], 1)
        self.assertFalse(result['days'][0]['complete'])
        self.assertNotIn('缺盘', result['days'][0]['status'])

    def test_unauthorized_rows_stop_and_report_contains_distributions(self):
        bad = bundle([record('1')])
        bad['data']['orders'][0]['shopCode'] = 'OUTSIDE'
        with self.assertRaises(ValueError):
            analysis.analyze(bad, today='2026-10-13')
        result = analysis.analyze(bundle([record('1')]), today='2026-10-13')
        report = analysis.report(result)
        self.assertIn('各范围准确率分布', report)
        self.assertIn('华东', report)
        self.assertIn('构造数据', report)


class ResumeTest(unittest.TestCase):
    def args(self):
        import argparse
        return argparse.Namespace(cli='fixture-cli', profile='fixture')

    def setup_query(self, root):
        contract = {'operationId': 'supply-chain-server.predict.shops.read', 'operationVersion': '1.0.0', 'contractDigest': 'a' * 64, 'effect': 'read'}
        state = {'queries': {}, 'context': {'queryLimit': 10}, 'sources': []}
        record = {'permission': {'contracts': [contract]}}
        return runner.Queries(self.args(), root, state, record), contract

    def test_public_cli_envelope_retains_contract_provenance(self):
        reply = {'ok': True, 'data': {'state': 'succeeded', 'complete': True, 'items': []},
                 'meta': {'contractVersion': '1.0.0', 'contractDigest': 'a' * 64}}
        process = subprocess.CompletedProcess([], 0, stdout=json.dumps(reply), stderr='diagnostic text')
        with patch.object(runner.subprocess, 'run', return_value=process):
            result = runner.cli('fixture-cli', ['jobs', 'wait', 'original'])
        self.assertTrue(result['complete'])
        self.assertEqual(result['_meta'], reply['meta'])

    def test_cached_data_rechecks_permission_before_yielding(self):
        with tempfile.TemporaryDirectory() as folder:
            queries, contract = self.setup_query(Path(folder))
            calls = []
            def okay(executable, argv):
                calls.append(argv)
                if argv[0] == 'schema': return contract
                if argv[:2] == ['jobs', 'submit']: return {'jobId': 'job-1', 'state': 'queued'}
                return {'jobId': 'job-1', 'state': 'succeeded', 'complete': True, 'items': [], '_meta': {'contractDigest': contract['contractDigest'], 'contractVersion': '1.0.0'}}
            with patch.object(runner, 'cli', okay):
                self.assertEqual(queries.get('shops', {'brandId': 1}), [])
            def denied(executable, argv):
                self.assertEqual(argv[0], 'schema')
                raise runner.CliError('SCOPE_DENIED')
            with patch.object(runner, 'cli', denied):
                self.assertIsNone(queries.get('shops', {'brandId': 1}))
            self.assertIn('SCOPE_DENIED', ' '.join(queries.issues))

    def test_continuation_refreshes_store_scope_even_when_contract_stays_available(self):
        with tempfile.TemporaryDirectory() as folder:
            queries, contract = self.setup_query(Path(folder))
            jobs = {}
            def execute(executable, argv):
                if argv[0] == 'schema':
                    return contract
                if argv[:2] == ['jobs', 'submit']:
                    identity = str(len(jobs) + 1)
                    jobs[identity] = [{'shopCode': 'S1'}, {'shopCode': 'S2'}] if identity == '1' else [{'shopCode': 'S1'}]
                    return {'jobId': identity, 'state': 'queued'}
                return {'state': 'succeeded', 'complete': True, 'items': jobs[argv[2]],
                        '_meta': {'contractDigest': contract['contractDigest'], 'contractVersion': '1.0.0'}}
            with patch.object(runner, 'cli', execute):
                self.assertEqual(len(queries.current_shops({'brandId': 1})), 2)
                resumed = runner.Queries(self.args(), Path(folder), queries.state, queries.record)
                self.assertEqual(resumed.current_shops({'brandId': 1}), [{'shopCode': 'S1'}])
            self.assertEqual(len(jobs), 2)
            self.assertEqual(resumed.state['submittedQueries'], 2)
            self.assertEqual(len(resumed.state['sources']), 2)

    def test_local_cached_result_requires_fresh_permission_and_keeps_original_job(self):
        with tempfile.TemporaryDirectory() as folder:
            queries, contract = self.setup_query(Path(folder))
            contract['operationVersion'] = '1.1.0'
            schema = {**contract, 'direct': {'schemaVersion': 1, 'execution': {'engine': 'direct-read'}}}
            calls = []
            def execute(executable, argv):
                calls.append(argv)
                if argv[0] == 'schema': return schema
                if argv[:2] == ['jobs', 'submit']: return {'jobId': 'local-original', 'state': 'queued', 'executionLocation': 'local'}
                return {'jobId': 'local-original', 'state': 'succeeded', 'complete': True, 'items': [{'shopCode': 'S1', 'revenue': 10.12}],
                        'executionLocation': 'local', '_meta': {'contractDigest': contract['contractDigest'], 'contractVersion': '1.1.0'}}
            with patch.object(runner, 'cli', execute):
                self.assertEqual(queries.get('shops', {'brandId': 1})[0]['revenue'], 10.12)
                calls.clear()
                self.assertEqual(str(queries.get('shops', {'brandId': 1})[0]['revenue']), '10.12')
            self.assertEqual([call[:2] for call in calls], [['schema', contract['operationId']], ['jobs', 'result']])
            def denied(executable, argv):
                if argv[0] == 'schema': return schema
                self.assertEqual(argv[:3], ['jobs', 'result', 'local-original'])
                raise runner.CliError('DATA_SCOPE_DENIED')
            with patch.object(runner, 'cli', denied):
                self.assertIsNone(queries.get('shops', {'brandId': 1}))
            self.assertIn('DATA_SCOPE_DENIED', ' '.join(queries.issues))
            self.assertEqual(next(iter(queries.state['queries'].values()))['jobId'], 'local-original')

    def test_unfinished_scope_check_resumes_its_original_job(self):
        with tempfile.TemporaryDirectory() as folder:
            queries, contract = self.setup_query(Path(folder))
            def interrupted(executable, argv):
                if argv[0] == 'schema': return contract
                if argv[:2] == ['jobs', 'submit']: return {'jobId': 'scope-original', 'state': 'queued'}
                raise runner.CliError('JOB_WAIT_TIMEOUT')
            with patch.object(runner, 'cli', interrupted):
                self.assertIsNone(queries.current_shops({'brandId': 1}))
            resumed = runner.Queries(self.args(), Path(folder), queries.state, queries.record)
            def completed(executable, argv):
                if argv[0] == 'schema': return contract
                self.assertNotEqual(argv[:2], ['jobs', 'submit'])
                self.assertEqual(argv[2], 'scope-original')
                if argv[:2] == ['jobs', 'status']: return {'state': 'succeeded'}
                return {'state': 'succeeded', 'complete': True, 'items': [{'shopCode': 'S1'}],
                        '_meta': {'contractDigest': contract['contractDigest'], 'contractVersion': '1.0.0'}}
            with patch.object(runner, 'cli', completed):
                self.assertEqual(resumed.current_shops({'brandId': 1}), [{'shopCode': 'S1'}])
            self.assertEqual(resumed.state['submittedQueries'], 1)

    def test_saved_resource_digest_prevents_new_rules_from_entering_old_run(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            resources = root / 'resources'; resources.mkdir()
            (resources / 'rule.py').write_bytes(b'old')
            record = {'resourcesDirectory': str(resources), 'package': {'files': {'rule.py': hashlib.sha256(b'old').hexdigest()}}}
            self.assertEqual(runner.verify_resources(root, record), resources.resolve())
            (resources / 'rule.py').write_bytes(b'new')
            with self.assertRaises(ValueError): runner.verify_resources(root, record)

    def test_new_task_enters_updated_rules_and_each_client_reads_its_own_version(self):
        from argparse import Namespace
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            resources = root / 'resources'
            shutil.copytree(SCRIPTS.parent, resources, ignore=shutil.ignore_patterns('__pycache__'))
            rules = resources / 'scripts/rules.py'
            rules.write_text(rules.read_text().replace('SEGMENT_DAYS = 31', 'SEGMENT_DAYS = 2')
                             .replace("CALCULATION_VERSION = '1.0.0'", "CALCULATION_VERSION = '1.1.0'"))
            entry = resources / 'SKILL.md'
            entry.write_text(entry.read_text().replace('version: "1.0.0"', 'version: "1.1.0"'))
            files = {str(file.relative_to(resources)): hashlib.sha256(file.read_bytes()).hexdigest()
                     for file in resources.rglob('*') if file.is_file()}
            record = {'runId': 'new-version', 'profile': 'fixture', 'resourcesDirectory': str(resources),
                      'skillVersion': '1.1.0', 'cliVersion': '0.6.0', 'package': {'files': files},
                      'permission': {'subjectId': '1', 'environment': 'local', 'systemId': 'supply-chain-server',
                                     'authorizationId': 'fixture-authorization'}}
            (root / 'run.json').write_text(json.dumps(record))
            args = Namespace(cli='old-cli', client='Codex', profile='fixture', brand=1, period=None,
                             start='2026-10-05', end='2026-10-11', shop=[], region=[], warehouse_group=[],
                             query_limit=None, skill_directory=str(root / 'installed'))
            with patch.object(runner, 'cli', return_value={'runDirectory': str(root), 'cliExecutable': 'updated-cli'}):
                self.assertEqual(runner.new_task(args), 0)
            state = json.loads((root / 'analysis-state.json').read_text())
            self.assertEqual(len(state['context']['segments']), 4)
            self.assertEqual(state['calculationVersion'], '1.1.0')
            self.assertEqual(state['cliExecutable'], 'updated-cli')
            state['clientReads'] = {'Codex': {'skillVersion': '1.1.0'}}
            state['clientReadConfirmed'] = True
            (root / 'analysis-state.json').write_text(json.dumps(state))
            other_client = subprocess.run([sys.executable, str(resources / 'scripts/run.py'), '--resume',
                                           str(root), '--profile', 'fixture', '--client', 'WorkBuddy',
                                           '--cli', 'unavailable-cli'], capture_output=True, text=True)
            self.assertEqual(other_client.returncode, 0, other_client.stderr)
            self.assertEqual(json.loads(other_client.stdout)['state'], 'needs-client-read')

    def test_download_interruption_keeps_the_original_job_for_resume(self):
        with tempfile.TemporaryDirectory() as folder:
            queries, contract = self.setup_query(Path(folder))
            def interrupted(executable, argv):
                if argv[0] == 'schema': return contract
                if argv[:2] == ['jobs', 'submit']: return {'jobId': 'original', 'state': 'queued'}
                raise runner.CliError('JOB_WAIT_TIMEOUT')
            with patch.object(runner, 'cli', interrupted):
                self.assertIsNone(queries.get('shops', {'brandId': 1}))
            resumed = runner.Queries(self.args(), Path(folder), queries.state, queries.record)
            calls = []
            def completed(executable, argv):
                calls.append(argv)
                if argv[0] == 'schema': return contract
                if argv[:2] == ['jobs', 'status']: return {'jobId': 'original', 'state': 'succeeded'}
                return {'state': 'succeeded', 'complete': True, 'items': [],
                        '_meta': {'contractDigest': contract['contractDigest'], 'contractVersion': '1.0.0'}}
            with patch.object(runner, 'cli', completed):
                self.assertEqual(resumed.get('shops', {'brandId': 1}), [])
            self.assertFalse(any(call[:2] == ['jobs', 'submit'] for call in calls))
            self.assertEqual(resumed.state['submittedQueries'], 1)

    def test_expired_read_has_a_bounded_retry_and_preserves_old_job_id(self):
        with tempfile.TemporaryDirectory() as folder:
            queries, contract = self.setup_query(Path(folder))
            digest = hashlib.sha256(analysis.stable({'operation': contract, 'arguments': {'brandId': 1}}).encode()).hexdigest()
            queries.state['queries'][digest] = {'state': 'running', 'jobId': 'expired', 'attempts': 1}
            queries.state['submittedQueries'] = queries.count = 1
            calls = []
            def completed(executable, argv):
                calls.append(argv)
                if argv[0] == 'schema': return contract
                if argv[:2] == ['jobs', 'status']:
                    return {'state': 'succeeded', 'resultExpiresAt': '2020-01-01T00:00:00Z'}
                if argv[:2] == ['jobs', 'submit']: return {'jobId': 'replacement', 'state': 'queued'}
                return {'state': 'succeeded', 'complete': True, 'items': [],
                        '_meta': {'contractDigest': contract['contractDigest'], 'contractVersion': '1.0.0'}}
            with patch.object(runner, 'cli', completed):
                self.assertEqual(queries.get('shops', {'brandId': 1}), [])
            query = queries.state['queries'][digest]
            self.assertEqual(query['previousJobs'][0]['jobId'], 'expired')
            self.assertEqual((query['attempts'], queries.state['submittedQueries']), (2, 2))

    def test_resume_preserves_query_limit_but_can_download_an_existing_job(self):
        with tempfile.TemporaryDirectory() as folder:
            queries, contract = self.setup_query(Path(folder))
            queries.state['submittedQueries'] = queries.count = queries.state['context']['queryLimit']
            with patch.object(runner, 'cli', lambda executable, argv: contract):
                self.assertIsNone(queries.get('shops', {'brandId': 1}))
            self.assertIn('QUERY_LIMIT_REACHED', ' '.join(queries.issues))

    def test_nested_brand_name_is_not_used_as_region(self):
        tree = [{'name': '品牌根节点', 'children': [{'name': '华东大区', 'shopInfoList': [{'id': 3}]}]}]
        self.assertEqual(runner.partition_regions(tree), {'3': {'华东大区'}})


class FullFlowTest(unittest.TestCase):
    def test_all_read_steps_produce_the_same_report_for_both_clients(self):
        fixture = ROOT / 'tests/business/fixtures/predict-responses.json'
        sample = json.loads(fixture.read_text())['responses']
        shops = [{'id': 3, 'shopCode': '2021003', 'shopName': '模拟门店甲'}, {'id': 4, 'shopCode': '2021004', 'shopName': '模拟门店乙'}]
        requirements = json.loads((SCRIPTS / 'requirements.json').read_text())
        from argparse import Namespace
        outputs = []
        for client, version in [('Codex', '1.0.0'), ('WorkBuddy', '1.0.0'), ('Codex', '1.1.0'), ('WorkBuddy', '1.1.0')]:
            contracts = [{**row, 'operationVersion': version, 'contractDigest': 'a' * 64} for row in requirements['operations']]
            with tempfile.TemporaryDirectory() as folder:
                state = {'runId': 'local-flow', 'queries': {}, 'sources': [], 'skillVersion': version,
                         'context': {'subjectId': '1', 'environment': 'local', 'brandId': 1, 'queryLimit': 1000,
                                     'startDate': '2026-10-05', 'endDate': '2026-10-11', 'shopCodes': [], 'regions': [],
                                     'segments': [{'startDate': '2026-10-05', 'endDate': '2026-10-11'}]}}
                record = {'permission': {'contracts': contracts}}
                jobs = {}
                def execute(executable, argv):
                    if argv[0] == 'schema':
                        schema = next(row.copy() for row in contracts if row['operationId'] == argv[1])
                        if version == '1.1.0': schema['direct'] = {'schemaVersion': 1, 'execution': {'engine': 'direct-read'}}
                        return schema
                    if argv[:2] == ['jobs', 'submit']:
                        op = argv[2].split('.predict.')[1][:-5]
                        arguments = json.loads(argv[argv.index('--params') + 1])
                        rows = copy.deepcopy(shops if op == 'shops' else sample.get(op, []))
                        if op == 'warehouse': rows = [sample['warehouse-a' if arguments['warehouseId'] == 31 else 'warehouse-b']]
                        for row in rows:
                            if op == 'shop-teams': row['shopCode'] = row.pop('code')
                        if 'shopCodes' in arguments:
                            rows = [row for row in rows if not row.get('shopCode') or row['shopCode'] in arguments['shopCodes']]
                        if 'startDate' in arguments:
                            rows = [row for row in rows if arguments['startDate'] <= row.get('orderDate', row.get('statisticDate', row.get('predictDay', row.get('inventoryDate', arguments['startDate'])))) <= arguments['endDate']]
                        identity = str(len(jobs) + 1)
                        jobs[identity] = {'state': 'succeeded', 'complete': True, 'items': rows,
                                          'executionLocation': 'local' if version == '1.1.0' else 'iam',
                                          '_meta': {'contractDigest': 'a' * 64, 'contractVersion': version}}
                        return {'jobId': identity, 'state': 'queued'}
                    return jobs[argv[2]]
                with patch.object(runner, 'cli', execute):
                    output = runner.run_queries(Namespace(cli='fixture', profile='fixture', client=client), Path(folder), state, record)
                self.assertEqual(output['state'], 'completed')
                result = json.loads((Path(folder) / 'report/result.json').read_text())
                self.assertEqual(result['metrics']['predictOrderAccuracy']['validCount'], 14)
                self.assertEqual(result['rankMode'], '观察排序')
                self.assertIn('本地模拟', result['sourceKind'])
                self.assertTrue(result['focusTargets'])
                self.assertTrue(result['dishEvidence'])
                outputs.append(result['metrics'])
        for output in outputs[1:]: self.assertEqual(outputs[0], output)


if __name__ == '__main__':
    unittest.main()
