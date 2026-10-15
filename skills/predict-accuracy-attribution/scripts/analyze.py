#!/usr/bin/env python3
"""读取受控数据文件，计算已确认规则，写入中文报告与结构化结果。仅使用 Python 标准库。"""
import argparse
from collections import Counter, defaultdict
from datetime import date, datetime, timedelta
from decimal import Decimal, InvalidOperation
import hashlib
import json
from pathlib import Path
from string import Template
from zoneinfo import ZoneInfo

from rules import (ACCURACY_THRESHOLDS, ANOMALY_RATIO, BASELINE_VALID_DAYS, BUSINESS_TIMEZONE,
                   CALCULATION_VERSION, CLASSIFICATION_VERSION, COMPARABLE_PERCENTAGE_POINTS,
                   CONFLICT, FRESH_CLASSES, METRICS, NON_FRESH, ORDER_PASS_RATE,
                   RESULT_SCHEMA, REVENUE_PASS_RATE, REVENUE_THRESHOLDS, SEGMENT_DAYS,
                   TEN_THOUSAND, TOP_ITEM_COUNT, UNKNOWN, WORST_SHOP_COUNT, MAX_SOURCE_BYTES)

ROOT = Path(__file__).resolve().parents[1]


def number(value):
    if value is None or isinstance(value, bool):
        return None
    try:
        value = Decimal(str(value))
        return value if value.is_finite() else None
    except (InvalidOperation, ValueError):
        return None


def positive(value):
    value = number(value)
    return value if value is not None and value > 0 else None


def day(value):
    try:
        return date.fromisoformat(str(value)[:10])
    except (ValueError, TypeError):
        return None


def dates(expression=None, start=None, end=None, today=None):
    today = day(today) if today else datetime.now(ZoneInfo(BUSINESS_TIMEZONE)).date()
    if expression == '上周':
        end = today - timedelta(days=today.weekday() + 1)
        start = end - timedelta(days=6)
    elif expression in ('近7天', '近 7 天'):
        end = today - timedelta(days=1)
        start = end - timedelta(days=6)
    else:
        start, end = day(start), day(end)
    if not today or not start or not end or start > end:
        raise ValueError('请提供明确的起止日期，或“上周”“近7天”。')
    segments = []
    current = start
    while current <= end:
        last = min(end, current + timedelta(days=SEGMENT_DAYS - 1))
        segments.append({'startDate': current.isoformat(), 'endDate': last.isoformat()})
        current = last + timedelta(days=1)
    return start, end, segments


def stable(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':'), default=str)


def unique(records, key='recordId'):
    """源 ID 相同才去重；同 ID 内容变化整组排除，展示相同的不同记录仍保留。"""
    seen, changed, missing, duplicates = {}, set(), [], 0
    for row in records:
        identity = row.get(key)
        if identity is None or str(identity) == '':
            missing.append(dict(row))
            continue
        identity = str(identity)
        if identity in seen:
            duplicates += 1
            if stable(seen[identity]) != stable(row):
                changed.add(identity)
        else:
            seen[identity] = dict(row)
    result = [row for identity, row in seen.items() if identity not in changed] + missing
    return result, {'duplicates': duplicates, 'changedIds': sorted(changed), 'missingIds': len(missing)}


def classification(items, groups, backend, mapping):
    seed = {str(row['itemCode']): row['classification'] for row in mapping['items']}
    current = {str(row.get('itemCode', row.get('code'))): row for row in items}
    parents = {}

    def find(code):
        parents.setdefault(code, code)
        if parents[code] != code:
            parents[code] = find(parents[code])
        return parents[code]

    def join(a, b):
        parents[find(b)] = find(a)

    checked = set()
    for group in groups:
        if group.get('complete') is not True:
            continue
        code = str(group.get('itemCode', ''))
        members = [str(member) for member in group.get('memberCodes', [])]
        if not code or code not in current:
            continue
        checked.add(code)
        for member in members:
            if member in current:
                join(code, member)
    members = defaultdict(set)
    for code in set(current) | set(parents):
        members[find(code)].add(code)
    backend_values = defaultdict(set)
    for row in backend:
        code = str(row.get('itemCode', ''))
        value = row.get('classification')
        if value in (*FRESH_CLASSES, NON_FRESH):
            backend_values[code].add(value)
    result = {}
    for code in current:
        values = {seed[member] for member in members[find(code)] if member in seed}
        backend_set = backend_values[code]
        if len(values) > 1 or len(backend_set) > 1 or values and backend_set and values != backend_set:
            category, source = CONFLICT, 'v2清单、同组或后台分类互相矛盾'
        elif code in seed:
            category, source = seed[code], '附件84项分类表v2直接维护'
        elif values and code in checked:
            category, source = next(iter(values)), '由完整同组查询继承，非直接维护'
        elif code in checked and not values:
            # The complete membership lookup provides the missing exclusion proof; the backend fallback alone does not.
            if backend_set and backend_set != {NON_FRESH}:
                category, source = CONFLICT, '后台鲜品分类与v2及完整同组核对矛盾'
            else:
                category, source = NON_FRESH, '当前品项及完整同组核对，未命中v2鲜品清单'
        else:
            category, source = UNKNOWN, '同组资料不足，后台默认非鲜品不能单独确认分类'
        result[code] = {'classification': category, 'source': source, 'version': CLASSIFICATION_VERSION,
                        'members': sorted(members[find(code)]), 'backend': sorted(backend_set)}
    return result


def distribution(values, thresholds=ACCURACY_THRESHOLDS, pass_rate=ORDER_PASS_RATE):
    values = [value for value in values if value is not None and 0 <= value <= 1]
    counts = [0] * (len(thresholds) + 1)
    for value in values:
        index = next((i for i, threshold in enumerate(thresholds) if value >= threshold), len(thresholds))
        counts[index] += 1
    labels = ['≥' + percent(thresholds[0])] + [f'[{percent(right)},{percent(left)})'
             for left, right in zip(thresholds, thresholds[1:])] + ['<' + percent(thresholds[-1])]
    passed = sum(value >= pass_rate for value in values)
    return {'validCount': len(values), 'bins': dict(zip(labels, counts)), 'passedCount': passed,
            'passShare': Decimal(passed) / len(values) if values else None}


def percent(value):
    value = number(value)
    return '无法计算' if value is None else f'{value * 100:.2f}%'


def valid_metric(row, metric):
    if row.get("recordId") is None or str(row.get("recordId")) == "":
        return None, "缺少可靠原记录编号"
    value = number(row.get(metric))
    _, predicted, actual = METRICS[metric]
    before, after = number(row.get(predicted)), number(row.get(actual))
    if before is None or after is None:
        return None, '原始比较值缺失'
    if before < 0 or after < 0:
        return None, '原始比较值为负'
    if value is None:
        return None, '准确率缺失'
    if value < 0 or value > 1:
        return None, '准确率超出0至1'
    return value, None


def join_records(indicators, orders):
    fields = ('shopCode', 'orderDate', 'arrivalDate', 'orderPeriod', 'itemCode', 'orderUnit')
    index = defaultdict(list)
    normalized_orders = []
    for row in orders:
        row = dict(row)
        if row.get('recordId') is None and row.get('id') is not None:
            row['recordId'] = str(row['id'])
        normalized_orders.append(row)
    normalized_orders, order_quality = unique(normalized_orders)
    for row in normalized_orders:
        index[tuple(str(row.get(field, '')) for field in fields)].append(row)
    result = []
    for row in indicators:
        row = dict(row)
        if row.get('recordId') is None:
            matched = index[tuple(str(row.get(field, '')) for field in fields)]
            if len(matched) == 1 and matched[0].get('recordId') is not None:
                row['recordId'] = matched[0]['recordId']
                row['identitySource'] = '精确单店请求和唯一原预估记录核对'
                row['cycleEnd'] = matched[0].get('cycleEnd')
                row['specification'] = matched[0].get('specification')
                row['inventoryAnchorsVerified'] = matched[0].get('inventoryAnchorsVerified')
            else:
                row['identitySource'] = '无法确定唯一原记录，保留数据并排除相应统计'
        result.append(row)
    result, indicator_quality = unique(result)
    return result, normalized_orders, {'indicators': indicator_quality, 'orders': order_quality}


def cycle_end(row):
    explicit = day(row.get('cycleEnd'))
    if explicit:
        return explicit
    start = day(row.get('arrivalDate'))
    period = number(row.get('orderPeriod'))
    if start and period is not None and period == int(period) and period > 0:
        return start + timedelta(days=int(period) - 1)
    return None


def worst(rows, count):
    eligible = [row for row in rows if row.get('passShare') is not None]
    eligible.sort(key=lambda row: (row['passShare'], str(row.get('shopCode'))))
    if len(eligible) <= count:
        return eligible
    cutoff = eligible[count - 1]['passShare']
    return [row for row in eligible if row['passShare'] <= cutoff]


def top_consumption(orders, classes, master):
    pools, windows, failures = defaultdict(lambda: defaultdict(Decimal)), defaultdict(list), defaultdict(set)
    for row in orders:
        code = str(row.get('itemCode', ''))
        if classes.get(code, {}).get('classification') != NON_FRESH:
            continue
        unit = row.get('orderUnit')
        specification = row.get('specification') or master.get(code, {}).get('specification')
        if not unit or not specification:
            failures['单位或规格缺失'].add(code)
            continue
        pool = (str(unit), str(specification))
        quantity = number(row.get('actualConsumeNum'))
        start, end = day(row.get('arrivalDate')), cycle_end(row)
        if quantity is None or quantity < 0 or not start or not end or row.get('recordId') is None:
            failures[pool].add('实际耗用、日期窗口或原记录编号不足')
            continue
        key = (row.get('shopCode'), code, *pool)
        if any(start <= previous_end and previous_start <= end for previous_start, previous_end in windows[key]):
            failures[pool].add('周期耗用日期重叠，无法可靠排除重复')
        windows[key].append((start, end))
        pools[pool][code] += quantity
    result = []
    for pool in sorted(set(pools) | {key for key in failures if isinstance(key, tuple)}):
        error = sorted(failures[pool])
        ranked = sorted(pools[pool].items(), key=lambda pair: (-pair[1], pair[0]))
        cutoff = ranked[TOP_ITEM_COUNT - 1][1] if len(ranked) >= TOP_ITEM_COUNT else None
        chosen = [(code, quantity) for code, quantity in ranked if cutoff is None or quantity >= cutoff]
        result.append({'unit': pool[0], 'specification': pool[1], 'trusted': not error,
                       'issues': error, 'items': [] if error else [{'itemCode': code, 'actualConsume': quantity} for code, quantity in chosen]})
    if failures.get('单位或规格缺失'):
        result.append({'unit': None, 'specification': None, 'trusted': False,
                       'issues': ['单位或规格缺失：' + '、'.join(sorted(failures['单位或规格缺失']))], 'items': []})
    return result


def daily_anomalies(records, start, end):
    groups = defaultdict(dict)
    duplicates = set()
    for row in records:
        key = (row.get('shopCode'), str(row.get('itemCode')), row.get('orderUnit'), row.get('specification'))
        value_date = day(row.get('statisticDate'))
        if value_date is None:
            continue
        if value_date in groups[key] and stable(groups[key][value_date]) != stable(row):
            duplicates.add((key, value_date))
        groups[key][value_date] = row
    result = []
    for key, rows in groups.items():
        valid = {}
        for value_date, row in rows.items():
            revenue, consume = positive(row.get('actualRevenue')), number(row.get('theoryConsume'))
            if revenue and consume is not None and consume >= 0 and (key, value_date) not in duplicates:
                valid[value_date] = consume / revenue * TEN_THOUSAND
        for value_date in sorted(d for d in rows if start <= d <= end):
            current = valid.get(value_date)
            history = sorted((d for d in valid if d < value_date), reverse=True)[:BASELINE_VALID_DAYS]
            baseline = sum((valid[d] for d in history), Decimal(0)) / len(history) if history else None
            deviation = (current - baseline) / baseline if current is not None and baseline and baseline > 0 else None
            result.append({'shopCode': key[0], 'itemCode': key[1], 'unit': key[2], 'specification': key[3],
                           'date': value_date.isoformat(), 'tenThousandConsume': current, 'baseline': baseline,
                           'sampleCount': len(history), 'baselineDates': [d.isoformat() for d in sorted(history)],
                           'deviation': deviation, 'anomaly': deviation is not None and abs(deviation) > ANOMALY_RATIO,
                           'reason': '基准为零或缺失，无法计算百分比' if deviation is None else None})
    return result


def dish_evidence(anomalies, records, daily):
    revenue = {(row.get('shopCode'), str(row.get('statisticDate'))): positive(row.get('actualRevenue')) for row in daily}
    queries = {}
    for query in records:
        key = (query.get('shopCode'), str(query.get('itemCode')), str(query.get('date')))
        if query.get('complete') is True:
            queries[key] = query.get('items', [])
    result = []
    for anomaly in (row for row in anomalies if row['anomaly']):
        key = (anomaly['shopCode'], anomaly['itemCode'])
        current = queries.get((*key, anomaly['date']))
        if current is None:
            result.append({**key_dict(key), 'date': anomaly['date'], 'issues': ['未取得完整异常日菜品耗用'], 'dishes': []})
            continue
        dishes = []
        for dish in current:
            identity = (dish.get('dishSku'), dish.get('dishSpu'), dish.get('recipeCode'), dish.get('dishSpecification'))
            amount = number(dish.get('consumeOrderUnit'))
            today_revenue = revenue.get((key[0], anomaly['date']))
            current_value = amount / today_revenue * TEN_THOUSAND if amount is not None and today_revenue else None
            samples = []
            for value_date in anomaly['baselineDates']:
                previous = queries.get((*key, value_date))
                sale = revenue.get((key[0], value_date))
                if previous is None or not sale:
                    continue
                matches = [row for row in previous if (row.get('dishSku'), row.get('dishSpu'), row.get('recipeCode'), row.get('dishSpecification')) == identity]
                amounts = [number(row.get('consumeOrderUnit')) for row in matches]
                if any(value is None for value in amounts):
                    continue
                samples.append(sum(amounts, Decimal(0)) / sale * TEN_THOUSAND)
            baseline = sum(samples, Decimal(0)) / len(samples) if samples else None
            deviation = (current_value - baseline) / baseline if current_value is not None and baseline and baseline > 0 else None
            dishes.append({'dishSku': identity[0], 'dishName': dish.get('dishName'), 'dishSpecification': identity[3],
                           'current': current_value, 'baseline': baseline, 'sampleCount': len(samples), 'deviation': deviation})
        result.append({**key_dict(key), 'date': anomaly['date'], 'issues': [], 'dishes': dishes})
    return result


def key_dict(key):
    return {'shopCode': key[0], 'itemCode': key[1]}


def inventory_summary(bills, rules, start, end):
    records, quality = unique(bills, 'inventoryNo')
    completed = defaultdict(set)
    for row in records:
        when = day(row.get('inventoryDate'))
        if when and start <= when <= end and row.get('type') == 'DAY_INVENTORY' and row.get('state') == 'FINISHED':
            completed[(row.get('shopCode'), when.isoformat())].add(str(row.get('warehouseId', '')))
    output = []
    for rule in rules:
        when = day(rule.get('date'))
        if not when or not start <= when <= end:
            continue
        ids = set(map(str, rule.get('requiredWarehouseIds', [])))
        observed = completed.get((rule.get('shopCode'), when.isoformat()), set())
        reliable = rule.get('historicalRuleVerified') is True and rule.get('allWarehousesVerified') is True and rule.get('operating') is True and bool(ids)
        missing = sorted(ids - observed) if reliable else []
        output.append({'shopCode': rule.get('shopCode'), 'date': when.isoformat(), 'complete': reliable and not missing,
                       'missingWarehouseIds': missing, 'status': ('缺盘' if missing else '全部应盘仓库完成') if reliable
                       else '历史应盘规则或全仓范围不足，仅能列出查到的日盘'})
    if not output:
        for (shop, when), warehouses in sorted(completed.items()):
            output.append({'shopCode': shop, 'date': when, 'complete': False, 'observedWarehouseIds': sorted(warehouses),
                           'status': '查到已完成日盘；历史应盘规则或全仓覆盖未核实'})
    return {'days': output, 'quality': quality, 'missingRuleEvidence': not rules}


def fresh_inventory_rule(row, configurations):
    when = day(row.get('orderDate'))
    matches = [config for config in configurations if config.get('historicalEffectiveVerified') is True
               and config.get('evidenceSource') and str(config.get('itemCode')) == str(row.get('itemCode'))
               and when and day(config.get('effectiveFrom')) and day(config.get('effectiveFrom')) <= when
               and (day(config.get('effectiveTo')) is None or when <= day(config.get('effectiveTo')))]
    if len(matches) != 1 or not isinstance(matches[0].get('fresh'), bool):
        return {'mode': 'unverified', 'description': '鲜活配置的历史适用范围未核实，库存差异保留观察值'}
    config = matches[0]
    return {'mode': 'fresh' if config['fresh'] else 'ordinary', 'description': '按有历史证据的鲜活库存规则核对' if config['fresh'] else '按有历史证据的普通库存规则核对',
            'source': config['evidenceSource'], 'configurationId': config.get('id')}


def attribution(targets, orders, anomalies, revenue, fresh_config):
    result = []
    for target in targets:
        shop = target['shopCode']
        affected = [row for row in orders if row.get('shopCode') == shop and (target.get('itemCode') is None or str(row.get('itemCode')) == target['itemCode'])]
        facts, candidates = [], []
        inventory_rules = []
        for row in affected:
            rule = fresh_inventory_rule(row, fresh_config)
            inventory_rules.append({'recordId': row.get('recordId'), **rule})
            predicted, actual = number(row.get('theoryInventory')), number(row.get('arrivalBeforeActualInventory'))
            if predicted is not None and actual is not None:
                facts.append({'kind': '库存差异', 'recordId': row.get('recordId'), 'itemCode': row.get('itemCode'),
                              'predicted': predicted, 'actual': actual, 'difference': predicted - actual,
                              'comparable': row.get('inventoryAnchorsVerified') is True and rule['mode'] != 'unverified',
                              'inventoryRule': rule,
                              'sourceFields': {'prediction': {key: row.get(key) for key in ('lastInventoryDate', 'inventory', 'inboundNum', 'outboundNum', 'theoryConsumeNum', 'inTransitNum', 'predictLossNum')},
                                               'recalculation': {key: row.get(key) for key in ('arrivalBeforeLastInventoryDate', 'arrivalBeforeInventory', 'arrivalBeforeInboundNum', 'arrivalBeforeOutboundNum', 'arrivalBeforeActualConsumeNum', 'arrivalBeforeInTransitNum', 'arrivalBeforeActualLossNum')}}})
                if predicted != actual:
                    candidates.append({'kind': '库存因素', 'status': '可能原因，待核对', 'recordId': row.get('recordId'),
                                       'explanation': '核对预估快照与事后回算的盘点起点、耗用、出入库和在途；无法对齐时不能直接拆解。'})
            loss_predicted, loss_actual = number(row.get('predictLossNum')), number(row.get('actualLossNum'))
            if loss_predicted is not None and loss_actual is not None and loss_predicted != loss_actual:
                facts.append({'kind': '模型耗损差异', 'recordId': row.get('recordId'), 'difference': loss_predicted - loss_actual,
                              'lossRate': row.get('lossRate'), 'source': '原接口模型回算，未证明真实实测耗损'})
                candidates.append({'kind': '耗损率或耗用变化', 'status': '可能原因，待核对',
                                   'explanation': '使用该预估记录保存的耗损率，核对其历史生效规则；模型回算不能证明实测耗损。'})
        for anomaly in anomalies:
            if anomaly['shopCode'] == shop and anomaly['anomaly'] and (target.get('itemCode') is None or anomaly['itemCode'] == target['itemCode']):
                facts.append({'kind': '万元耗用偏差', **anomaly})
                candidates.append({'kind': '耗用需求波动', 'status': '有数值差异，原因待核对', 'date': anomaly['date'],
                                   'explanation': '结合异常日和前14个有效营业额日的菜品耗用；天气、节假日和责任判断需要额外证据。'})
        for row in revenue:
            if row.get('shopCode') != shop:
                continue
            accuracy = number(row.get('predictAmountAccuracy'))
            if accuracy is not None and accuracy < REVENUE_PASS_RATE:
                effective = number(row.get('predictAmount'))
                if effective is None or effective == 0:
                    effective = number(row.get('backupPredictAmount'))
                actual = number(row.get('actualAmount'))
                facts.append({'kind': '营业额预估偏差', 'date': row.get('predictDay'), 'accuracy': accuracy,
                              'direction': '估多了' if effective is not None and actual is not None and effective > actual else '估少了',
                              'effectivePrediction': effective, 'actual': actual})
                candidates.append({'kind': '营业额预估波动', 'status': '可能原因，待核对', 'date': row.get('predictDay')})
        result.append({**target, 'facts': facts, 'candidates': candidates, 'confirmedCauses': [],
                       'inventoryRules': inventory_rules, 'freshRule': '；'.join(dict.fromkeys(rule['description'] for rule in inventory_rules)) or '鲜活配置的历史适用范围未核实'})
    return result


def analyze(bundle, today=None):
    context = bundle.get('context', {})
    start, end, segments = dates(start=context.get('startDate'), end=context.get('endDate'), today=today)
    now = day(today) if today else datetime.now(ZoneInfo(BUSINESS_TIMEZONE)).date()
    data = bundle.get('data', {})
    allowed = set(map(str, context.get('authorizedShopCodes', [])))
    requested = set(map(str, context.get('shopCodes', [])))
    if not context.get('subjectId') or not context.get('environment') or context.get('brandId') is None or not allowed:
        raise ValueError('分析记录必须包含本人、环境、品牌和有权门店范围。')
    if requested and not requested <= allowed:
        raise ValueError('分析门店不在记录中的有权范围内。')
    scope = requested or allowed
    def scoped(key, date_field=None):
        rows = []
        for row in data.get(key, []):
            if row.get('shopCode') not in scope:
                raise ValueError('数据返回了未请求或无权门店，停止计算。')
            when = day(row.get(date_field)) if date_field else None
            if not date_field or when and start <= when <= end:
                rows.append(row)
        return rows
    indicators, orders, quality = join_records(scoped('indicators', 'orderDate'), scoped('orders', 'orderDate'))
    master = {str(row.get('itemCode', row.get('code'))): row for row in data.get('items', [])}
    mapping = json.loads((ROOT / 'assets/five-fresh-mapping-v2.json').read_text())
    classes = classification(data.get('items', []), data.get('groups', []), data.get('classifications', []), mapping)
    shops = {row['shopCode']: row for row in data.get('shops', []) if row.get('shopCode') in scope}
    evidence = {str(row.get('recordId')) for row in data.get('writebackEvidence', []) if row.get('status') == 'complete'
                and row.get('source') and row.get('verifiedAt')}
    complete = bundle.get('coverage', {}).get('complete') is True
    metrics, groups = {}, defaultdict(list)
    maturity = Counter()
    for row in indicators:
        code, shop = str(row.get('itemCode')), row.get('shopCode')
        category = classes.get(code, {'classification': UNKNOWN})['classification']
        row['classification'] = category
        region = row.get('historicalRegion') if row.get('historicalRegionVerified') is True else shops.get(shop, {}).get('region', '大区未取得')
        row['region'] = region
        last = cycle_end(row)
        mature = bool(last and last < now and str(row.get('recordId')) in evidence and row.get('recordId') is not None)
        row['formalEligible'] = mature and complete
        maturity['formalEligible' if row['formalEligible'] else 'observationOnly'] += 1
        groups[('shop', shop)].append(row)
        groups[('region', region)].append(row)
        groups[('category', category)].append(row)
        groups[('region-category', region, category)].append(row)
        groups[('shop-category', shop, category)].append(row)
        groups[('shop-item', shop, code)].append(row)
    for metric, (title, _, _) in METRICS.items():
        values, excluded = [], Counter()
        for row in indicators:
            value, reason = valid_metric(row, metric)
            if reason:
                excluded[reason] += 1
            else:
                values.append(value)
        metrics[metric] = {'title': title, **distribution(values), 'inputCount': len(indicators),
                           'excludedCount': sum(excluded.values()), 'exclusions': dict(excluded)}
    group_results = []
    for key, rows in sorted(groups.items()):
        for metric in METRICS:
            values = [valid_metric(row, metric)[0] for row in rows]
            valid = [value for value in values if value is not None]
            group_results.append({'dimension': key[0], 'keys': list(key[1:]), 'metric': metric,
                                  **distribution(valid), 'inputCount': len(rows), 'excludedCount': len(rows) - len(valid),
                                  'formal': complete and all(row['formalEligible'] for row in rows)})
    shop_rank = [{'shopCode': row['keys'][0], **row} for row in group_results
                 if row['dimension'] == 'shop' and row['metric'] == 'predictOrderAccuracy']
    targets = {}
    def add_target(row, reason, category=None, item=None):
        key = (row['shopCode'], category, item)
        target = targets.setdefault(key, {'shopCode': row['shopCode'], 'category': category, 'itemCode': item,
                                          'formal': row['formal'], 'reasons': []})
        if reason not in target['reasons']:
            target['reasons'].append(reason)
    for row in worst(shop_rank, WORST_SHOP_COUNT):
        add_target(row, '订货量准确率达标占比最差3名门店（含并列）')
    for category in FRESH_CLASSES:
        ranks = [{'shopCode': row['keys'][0], **row} for row in group_results if row['dimension'] == 'shop-category'
                 and row['keys'][1] == category and row['metric'] == 'predictOrderAccuracy']
        for row in worst(ranks, 1):
            add_target(row, '该鲜品类别表现最差门店（含并列）', category=category)
    top = top_consumption(orders, classes, master)
    for pool in top:
        for item in pool['items']:
            ranks = [{'shopCode': row['keys'][0], **row} for row in group_results if row['dimension'] == 'shop-item'
                     and row['keys'][1] == item['itemCode'] and row['metric'] == 'predictOrderAccuracy']
            for row in worst(ranks, 1):
                add_target(row, '非鲜品实际耗用前5品项的最差门店（含并列）', item=item['itemCode'])
    revenue = scoped('revenue', 'predictDay')
    revenue_rows = [{**row, 'revenueKey': f'{row.get("shopCode")}:{row.get("predictDay")}'} for row in revenue]
    revenue, revenue_quality = unique(revenue_rows, 'revenueKey')
    revenue_stats = {}
    for name, accuracy_field, prediction in (('main', 'predictAmountAccuracy', 'predictAmount'), ('manual', 'adjustPredictAmountAccuracy', 'adjustPredictAmount')):
        values, excluded = [], Counter()
        for row in revenue:
            value, actual = number(row.get(accuracy_field)), positive(row.get('actualAmount'))
            forecast = number(row.get(prediction))
            if name == 'main' and (forecast is None or forecast == 0):
                forecast = number(row.get('backupPredictAmount'))
            if value is None or not actual or forecast is None or forecast < 0 or value < 0 or value > 1:
                excluded['准确率、实际营业额或对应预估值缺失或无效'] += 1
            else:
                values.append(value)
        revenue_stats[name] = {**distribution(values, REVENUE_THRESHOLDS, REVENUE_PASS_RATE), 'exclusions': dict(excluded),
                               'excludedCount': sum(excluded.values())}
    daily = scoped('dailyInventory')
    anomalies = daily_anomalies(daily, start, end)
    coverage = bundle.get('coverage', {})
    issues = list(coverage.get('issues', []))
    if not complete:
        issues.append('查询范围未全部完成，只报告明确标注范围的已完成部分')
    if not indicators or maturity['observationOnly']:
        issues.append('回写完成未核实；缺证据记录只提供观察统计或观察排序')
    for key, value in quality.items():
        if value['changedIds']:
            issues.append(f'{key}同一原记录内容发生变化，相关编号已排除：' + '、'.join(value['changedIds']))
        if value['missingIds']:
            issues.append(f'{key}有{value["missingIds"]}条记录缺少可靠编号')
    class_counts = Counter(row['classification'] for row in indicators)
    if class_counts[UNKNOWN] or class_counts[CONFLICT]:
        issues.append(f'分类未知{class_counts[UNKNOWN]}条，冲突{class_counts[CONFLICT]}条，分别列出')
    region_rows = [row for row in group_results if row['dimension'] == 'region-category' and row['metric'] == 'predictOrderAccuracy']
    comparisons = []
    for i, left in enumerate(region_rows):
        for right in region_rows[i + 1:]:
            if left['keys'][1] == right['keys'][1] and left['passShare'] is not None and right['passShare'] is not None:
                difference = (left['passShare'] - right['passShare']) * 100
                comparisons.append({'left': left['keys'][0], 'right': right['keys'][0], 'category': left['keys'][1],
                                    'differencePercentagePoints': difference, 'comparable': abs(difference) <= COMPARABLE_PERCENTAGE_POINTS})
    result = {'schemaVersion': RESULT_SCHEMA, 'calculationVersion': CALCULATION_VERSION, 'classificationVersion': CLASSIFICATION_VERSION,
              'context': context, 'sourceKind': bundle.get('sourceKind', '未标明'), 'sources': bundle.get('sources', []),
              'shopNames': {row['shopCode']: row.get('shopName', row.get('name')) for row in data.get('shops', []) if row.get('shopCode') in scope},
              'requestedSegments': segments, 'coverage': coverage, 'maturity': dict(maturity), 'quality': quality,
              'rankMode': '正式排名' if complete and indicators and not maturity['observationOnly'] else '观察排序',
              'regionBasis': '历史大区归属' if indicators and all(row.get('historicalRegionVerified') is True for row in indicators) else '按当前大区归属整理',
              'classification': classes, 'classificationRecordCounts': dict(class_counts), 'metrics': metrics, 'distributions': group_results,
              'regionComparisons': comparisons, 'shopRank': sorted(shop_rank, key=lambda row: (row['passShare'] is None, row['passShare'] or Decimal(0), row['shopCode'])),
              'focusTargets': list(targets.values()), 'nonFreshTop5': top, 'revenue': revenue_stats, 'revenueQuality': revenue_quality,
              'inventory': inventory_summary(scoped('inventoryHistory', 'inventoryDate'), data.get('inventoryRules', []), start, end),
              'dailyComparisons': anomalies, 'dishEvidence': dish_evidence(anomalies, data.get('dishConsume', []), daily),
              'attribution': attribution(list(targets.values()), orders, anomalies, revenue, data.get('freshConfig', [])), 'issues': list(dict.fromkeys(issues))}
    return result


def report(result):
    def shop_label(code):
        name = result.get('shopNames', {}).get(code)
        label = f'{name}（{code}）' if isinstance(name, str) and name.strip() else str(code)
        return label.replace('|', '\\|').replace('\n', ' ').replace('\r', ' ')

    metrics = '\n'.join(f'| {row["title"]} | {row["validCount"]} | {row["excludedCount"]} | {percent(row["passShare"])} |'
                        for row in result['metrics'].values())
    inventory = '\n'.join(f'- {shop_label(row["shopCode"])} {row["date"]}：{row["status"]}' for row in result['inventory']['days']) or '未查到已完成日盘记录；历史应盘规则和全仓覆盖未核实。'
    revenue = '\n'.join(f'- {"主预估（保留备用回退）" if key == "main" else "人工调整"}：有效{row["validCount"]}条，≥85%占比{percent(row["passShare"])}，排除{row["excludedCount"]}条。\n  分布：' + '；'.join(f'{key} {value}条' for key, value in row['bins'].items()) for key, row in result['revenue'].items())
    targets = '\n'.join(f'- 门店 {shop_label(row["shopCode"])}'+ (f'，{row["category"]}' if row.get('category') else '')
                        + (f'，品项 {row["itemCode"]}' if row.get('itemCode') else '') + '：' + '；'.join(row['reasons'])
                        for row in result['focusTargets']) or '当前有效数据不足以定位重点门店。'
    top = '\n'.join(f'- {row["unit"] or "单位不明"} / {row["specification"] or "规格不明"}：'
                    + ('；'.join(f'{item["itemCode"]} {item["actualConsume"]}' for item in row['items']) if row['trusted'] else '无法形成可信排名，' + '；'.join(row['issues'])) for row in result['nonFreshTop5']) or '没有已核实、单位规格可比较的非鲜品实际耗用。'
    attribution_text = []
    for row in result['attribution']:
        text = f'### 门店 {shop_label(row["shopCode"])}' + (f' / 品项 {row["itemCode"]}' if row.get('itemCode') else '')
        text += '\n\n' + row['freshRule'] + '。\n\n数值事实：\n'
        text += '\n'.join('- ' + fact_text(fact) for fact in row['facts']) or '- 没有可直接复核的差异。'
        text += '\n\n可能原因与待核对事项：\n'
        text += '\n'.join('- ' + item['kind'] + '：' + item.get('explanation', '需结合原记录和门店证据核对。') for item in row['candidates']) or '- 数据不足，暂不判断原因。'
        attribution_text.append(text)
    distribution_lines = []
    dimensions = {'shop': '门店', 'region': '大区', 'category': '类别', 'region-category': '大区和类别', 'shop-category': '门店和类别', 'shop-item': '门店和品项'}
    for row in result['distributions']:
        counts = list(row['bins'].values())
        keys = list(map(str, row['keys']))
        if row['dimension'].startswith('shop'):
            keys[0] = shop_label(keys[0])
        distribution_lines.append('| ' + ' | '.join([dimensions[row['dimension']], ' / '.join(keys), METRICS[row['metric']][0],
                                  str(row['validCount']), str(row['excludedCount']), *map(str, counts), percent(row['passShare'])]) + ' |')
    comparisons = '\n'.join(f'- {row["category"]}：{row["left"]}与{row["right"]}达标占比差{abs(row["differencePercentagePoints"]):.2f}个百分点，'
                            + ('相当。' if row['comparable'] else '存在差异。') for row in result['regionComparisons']) or '当前数据不足以比较大区差异。'
    dish_lines = []
    for row in result.get('dishEvidence', []):
        text=f'- 门店{shop_label(row["shopCode"])}，品项{row["itemCode"]}，{row["date"]}：'
        text+='；'.join(row['issues']) if row.get('issues') else '已核对异常日与历史有效营业日。'
        for dish in row.get('dishes', []):
            text+=f'\n  - {dish.get("dishName") or dish.get("dishSku")}：当日万元耗用{dish.get("current")}，此前{dish["sampleCount"]}个有效日均值{dish.get("baseline")}，偏差{percent(dish.get("deviation"))}。'
        dish_lines.append(text)
    template = Template((ROOT / 'templates/report.md').read_text())
    return template.substitute(start=result['context']['startDate'], end=result['context']['endDate'],
                               source=result['sourceKind'], rank=result['rankMode'], region=result['regionBasis'],
                               calculation=result['calculationVersion'], classification=result['classificationVersion'],
                               inventory=inventory, revenue=revenue, metrics=metrics, targets=targets, top=top,
                               distributions="\n".join(distribution_lines) or "| 数据不足 | — | — | 0 | 0 | 0 | 0 | 0 | 0 | 无法计算 |",
                               comparisons=comparisons, dish="\n".join(dish_lines) or "没有已完成的异常日菜品比较。",
                               attribution='\n\n'.join(attribution_text) or '没有可分析对象。',
                               issues='\n'.join('- ' + issue for issue in result['issues']) or '- 无已发现的数据缺口。')


def fact_text(fact):
    kind = fact['kind']
    if kind == '万元耗用偏差':
        return f'{fact["date"]} 品项{fact["itemCode"]}：万元耗用{fact["tenThousandConsume"]}，此前{fact["sampleCount"]}个有效日均值{fact["baseline"]}，偏差{percent(fact["deviation"])}。'
    if kind == '库存差异':
        return f'原记录{fact["recordId"]}：预估库存{fact["predicted"]}，回算实际库存{fact["actual"]}，差异{fact["difference"]}。' + ('起点与时间范围已核对。' if fact['comparable'] else '盘点起点与时间范围未核实，无法直接拆解。')
    if kind == '营业额预估偏差':
        return f'{fact["date"]}：{fact["direction"]}，服务端准确率{percent(fact["accuracy"])}。'
    return f'{kind}：差异{fact.get("difference")}。原接口模型回算，需另行核实实测耗损。'


def load(path):
    if path.is_symlink() or path.stat().st_size > MAX_SOURCE_BYTES:
        raise ValueError('数据文件不符合大小或文件类型要求。')
    return json.loads(path.read_text(), parse_float=Decimal)


def write_result(bundle, destination, today=None):
    result = analyze(bundle, today)
    destination.mkdir(parents=True, exist_ok=True, mode=0o700)
    structured = destination / 'result.json'
    structured.write_text(json.dumps(result, ensure_ascii=False, indent=2, default=str) + '\n')
    report_path = destination / 'report.md'
    report_path.write_text(report(result))
    summary = f'{result["context"]["startDate"]}至{result["context"]["endDate"]}，{result["rankMode"]}；重点对象{len(result["focusTargets"])}个。'
    if result['issues']:
        summary += ' ' + '；'.join(result['issues'][:3]) + '。'
    return {'summary': summary, 'report': str(report_path.resolve()), 'result': str(structured.resolve()),
            'resultSha256': hashlib.sha256(structured.read_bytes()).hexdigest(), 'rankMode': result['rankMode']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--today')
    args = parser.parse_args()
    try:
        print(json.dumps(write_result(load(args.input), args.output, args.today), ensure_ascii=False))
    except (ValueError, OSError) as error:
        parser.exit(1, str(error) + '\n')


if __name__ == '__main__':
    main()
