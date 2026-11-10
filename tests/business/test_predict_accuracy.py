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
runner = importlib.import_module('planning')


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


class PlanningTest(unittest.TestCase):
    def test_nested_brand_name_is_not_used_as_region(self):
        tree=[{'name':'华东大区','children':[{'name':'品牌','shopInfoList':[{'id':3}]}]}]
        self.assertEqual(runner.partition_regions(tree), {'3': {'华东大区'}})


if __name__ == '__main__':
    unittest.main()
