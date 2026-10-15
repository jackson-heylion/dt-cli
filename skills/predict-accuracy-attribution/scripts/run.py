#!/usr/bin/env python3
"""新任务准备、受控取数、按版本计算及继续原任务；执行命令使用参数数组。"""
import argparse
from datetime import datetime, timedelta, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

from analyze import analyze, dates, day, load, stable, write_result
from rules import (BASELINE_VALID_DAYS, BUSINESS_TIMEZONE, CALCULATION_VERSION, DATA_FILES,
                   JOB_WAIT_SECONDS, LOOKBACK_LIMIT_DAYS, MAIN_QUERY_LIMIT, MAX_SOURCE_BYTES,
                   OPERATIONS, OP_PREFIX, PROCESS_TIMEOUT_SECONDS, RESULT_SCHEMA, SEGMENT_DAYS,
                   MAX_READ_ATTEMPTS, RETRYABLE_READ_FAILURES, STATE_FILE, BUNDLE_FILE, REPORT_DIRECTORY)
from rules import (CLIENT_DIRECTORIES, DATA_DIRECTORY, ENTRY_FILE, RESOURCE_DIRECTORY,
                   RUN_RECORD, SKILL_NAME, SOURCE_LIVE, SOURCE_LOCAL)

ROOT = Path(__file__).resolve().parents[1]


class CliError(Exception):
    def __init__(self, code):
        self.code = code
        super().__init__(code)


def cli(executable, args):
    result = subprocess.run([str(executable), *map(str, args)], capture_output=True, text=True,
                            encoding='utf-8', timeout=PROCESS_TIMEOUT_SECONDS)
    if len(result.stdout.encode('utf-8')) > MAX_SOURCE_BYTES:
        raise CliError('CLI_RESPONSE_TOO_LARGE')
    try:
        body = json.loads(result.stdout)
    except ValueError:
        raise CliError('CLI_RESPONSE_INVALID')
    if not isinstance(body, dict):
        raise CliError('CLI_RESPONSE_INVALID')
    if result.returncode or body.get('ok') is not True:
        # Raw stderr or rejected input can contain private data. Preserve the public error code only.
        error = body.get('error')
        raise CliError(error.get('code', 'CLI_FAILED') if isinstance(error, dict) else 'CLI_FAILED')
    data = body.get('data')
    if isinstance(data, dict):
        # The public CLI envelope keeps provenance beside data; retain it for result checks.
        meta = body.get('meta', {})
        if not isinstance(meta, dict):
            raise CliError('CLI_RESPONSE_INVALID')
        data = {**data, '_meta': meta}
    return data


def atomic(path, data):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    if path.is_symlink():
        raise ValueError('运行记录不能是符号链接。')
    descriptor, filename = tempfile.mkstemp(dir=path.parent)
    try:
        with os.fdopen(descriptor, 'w', encoding='utf-8') as output:
            json.dump(data, output, ensure_ascii=False, indent=2, default=str)
            output.write('\n')
            output.flush()
            os.fsync(output.fileno())
        os.replace(filename, path)
    finally:
        if os.path.exists(filename):
            os.unlink(filename)


def initial_request(args):
    start, end, segments = dates(args.period, args.start, args.end)
    return {'startDate': start.isoformat(), 'endDate': end.isoformat(), 'segments': segments,
            'brandId': args.brand, 'shopCodes': list(dict.fromkeys(args.shop)), 'regions': list(dict.fromkeys(args.region)),
            'queryLimit': args.query_limit, 'warehouseGroups': list(dict.fromkeys(args.warehouse_group))}


def request_arguments(args):
    values = ['--brand', str(args.brand)]
    for option, value in (('--period', args.period), ('--start', args.start), ('--end', args.end),
                          ('--query-limit', args.query_limit)):
        if value is not None:
            values.extend([option, str(value)])
    for option, entries in (('--shop', args.shop), ('--region', args.region),
                            ('--warehouse-group', args.warehouse_group)):
        for value in entries:
            values.extend([option, str(value)])
    return values


def new_task(args):
    if args.brand is None or args.brand <= 0:
        raise ValueError('新任务需要明确的本人有权品牌编号。')
    installed = Path(args.skill_directory).expanduser().resolve()
    prepared = cli(args.cli, ['skill', 'prepare', '--name', SKILL_NAME, '--directory', installed, '--profile', args.profile])
    run_directory = Path(prepared['runDirectory'])
    resources = verify_resources(run_directory, load(run_directory / RUN_RECORD))
    args.cli = prepared.get('cliExecutable', args.cli)
    # Parse dates and split ranges inside the newly installed version, not the caller's old code.
    return cli_script(args, resources / 'scripts/run.py',
                      ['--resume', run_directory, '--profile', args.profile, '--initialize', *request_arguments(args)])


def initialize_task(args, run_directory, record):
    if (run_directory / STATE_FILE).exists():
        raise ValueError('本任务已创建，使用原记录继续。')
    if args.brand is None or args.brand <= 0:
        raise ValueError('新任务需要明确的本人有权品牌编号。')
    if args.profile != record['profile']:
        raise ValueError('新任务使用已准备记录绑定的 profile。')
    args.query_limit = MAIN_QUERY_LIMIT if args.query_limit is None else args.query_limit
    if not 1 <= args.query_limit <= MAIN_QUERY_LIMIT:
        raise ValueError(f'查询次数必须在1至{MAIN_QUERY_LIMIT}之间。')
    request = initial_request(args)
    state = {'schemaVersion': RESULT_SCHEMA, 'context': {**request, 'subjectId': record['permission']['subjectId'],
              'environment': record['permission']['environment'], 'systemId': record['permission']['systemId'],
              'authorizationId': record['permission']['authorizationId']}, 'profile': args.profile,
              'runId': record['runId'], 'skillVersion': record['skillVersion'], 'cliVersion': record['cliVersion'],
              'cliExecutable': args.cli, 'calculationVersion': CALCULATION_VERSION,
              'queries': {}, 'sources': [], 'completedStages': [], 'state': 'prepared',
              'clientReadConfirmed': False, 'client': args.client, 'submittedQueries': 0}
    atomic(run_directory / STATE_FILE, state)
    return state


def cli_script(args, script, argv):
    process = subprocess.run([sys.executable, str(script), '--cli', args.cli, '--client', args.client, *map(str, argv)])
    return process.returncode


def verify_resources(run_directory, record):
    resources = Path(record['resourcesDirectory'])
    if resources.resolve() != (run_directory / RESOURCE_DIRECTORY).resolve() or resources.is_symlink():
        raise ValueError('运行资料位置与原记录不一致。')
    for name, digest in record['package']['files'].items():
        file = resources / name
        if file.is_symlink() or not file.is_file() or hashlib.sha256(file.read_bytes()).hexdigest() != digest:
            raise ValueError('运行资料已被修改或缺失：' + name)
    return resources.resolve()


def check_identity(args, state, record):
    checked = cli(args.cli, ['auth', 'status', '--profile', args.profile])
    # sync performs the current server check; the profile must still belong to the same person and environment.
    synced = cli(args.cli, ['catalog', 'sync', '--profile', args.profile])
    profile = cli(args.cli, ['profiles', 'list', '--details', '--system', state['context']['systemId'], '--environment', state['context']['environment']])
    entries = profile.get('profiles', profile.get('items', [])) if isinstance(profile, dict) else profile
    match = [row for row in entries if row.get('profile') == args.profile]
    if len(match) != 1:
        raise ValueError('未找到原任务绑定的 profile。')
    actual = match[0]
    for field in ('subjectId', 'environment', 'systemId', 'authorizationId'):
        if actual.get(field) != state['context'].get(field):
            raise ValueError('当前本人、环境、系统或授权与原任务不一致。')
    return synced


class Queries:
    def __init__(self, args, run_directory, state, record):
        self.args, self.root, self.state, self.record = args, run_directory, state, record
        self.count = state.setdefault('submittedQueries', sum(row.get('attempts', int(bool(row.get('jobId'))))
                                                             for row in state['queries'].values()))
        self.issues = []
        self.contracts = {row['operationId']: row for row in record['permission']['contracts']}

    def save(self):
        atomic(self.root / STATE_FILE, self.state)

    def current_shops(self, arguments):
        # Store scope can shrink while the operation itself remains authorised. Reuse an
        # unfinished scope job; a completed check needs a new read on the next continuation.
        generation = self.state.get('scopeVerification', 0)
        previous = self.state['queries'].get(self.state.get('scopeQueryDigest'), {})
        if generation == 0 or previous.get('state') == 'succeeded':
            generation += 1
        self.state['scopeVerification'] = generation
        return self.get('shops', arguments, scope_check=generation)

    def get(self, kind, arguments, scope_check=None):
        operation = OP_PREFIX + OPERATIONS[kind]
        contract = self.contracts.get(operation)
        if not contract:
            self.issues.append(f'{kind}接口权限或版本不足')
            self.state['queries'].setdefault('missing-' + operation, {'operationId': operation, 'state': 'unavailable', 'arguments': arguments, 'failureCode': 'SCOPE_DENIED'})
            self.save()
            return None
        identity = {'operation': contract, 'arguments': arguments}
        if scope_check is not None:
            identity['scopeCheck'] = scope_check
        digest = hashlib.sha256(stable(identity).encode()).hexdigest()
        if scope_check is not None:
            self.state['scopeQueryDigest'] = digest
        query = self.state['queries'].setdefault(digest, {'operationId': operation, 'operationVersion': contract['operationVersion'],
                    'contractDigest': contract['contractDigest'], 'arguments': arguments, 'state': 'new'})
        result_file = self.root / DATA_DIRECTORY / (digest + '.json')
        try:
            current = cli(self.args.cli, ['schema', operation, '--version', contract['operationVersion'], '--profile', self.args.profile])
            current = current.get('schema', current)
            if current.get('contractDigest') != contract['contractDigest'] or current.get('effect') != 'read':
                raise CliError('CONTRACT_CHANGED')
            location = 'local' if isinstance(current.get('direct'), dict) else 'iam'
            if contract['operationVersion'] == '1.1.0' and location != 'local':
                raise CliError('EXECUTOR_UNAVAILABLE')
            if query.get('executionLocation', location) != location:
                raise CliError('CONTRACT_CHANGED')
            query['executionLocation'] = location
        except CliError as error:
            query['failureCode'] = error.code
            query['availability'] = 'unavailable'
            self.issues.append(f'{kind}原接口当前不可用：{error.code}')
            self.save()
            return None
        query['availability'] = 'available'
        if query['state'] == 'succeeded':
            if not result_file.is_file() or result_file.is_symlink() or result_file.stat().st_size > MAX_SOURCE_BYTES or hashlib.sha256(result_file.read_bytes()).hexdigest() != query.get('sha256'):
                raise ValueError('已有数据文件缺失或内容变化，不能直接恢复。')
            rows = load(result_file)
            if query['executionLocation'] == 'local':
                try:
                    verified = cli(self.args.cli, ['jobs', 'result', query['jobId'], '--profile', self.args.profile])
                    meta = verified.get('_meta', {})
                    # The analysis loader keeps Decimal values; compare the persisted JSON
                    # representation using the same decoder as the public CLI envelope.
                    original_json = json.loads(result_file.read_text())
                    if verified.get('complete') is not True or verified.get('items') != original_json:
                        raise CliError('PARTIAL_RESULT')
                    if meta.get('contractDigest') != contract['contractDigest'] or meta.get('contractVersion') != contract['operationVersion']:
                        raise CliError('CONTRACT_CHANGED')
                except CliError as error:
                    query['failureCode'] = error.code
                    query['availability'] = 'unavailable'
                    self.issues.append(f'{kind}原数据当前不能使用：{error.code}')
                    self.save()
                    return None
            return rows
        try:
            if query.get('jobId'):
                status = cli(self.args.cli, ['jobs', 'status', query['jobId'], '--profile', self.args.profile])
                query['observedState'] = status['state']
                query['state'] = 'running' if status['state'] == 'succeeded' else status['state']
                self.save()
                expired = False
                if status['state'] == 'succeeded' and status.get('resultExpiresAt'):
                    expired = datetime.fromisoformat(status['resultExpiresAt'].replace('Z', '+00:00')) <= datetime.now(timezone.utc)
                if status['state'] in ('failed', 'cancelled') or expired:
                    code = status.get('failureCode')
                    if expired:
                        code = 'RESULT_EXPIRED'
                    if status['state'] == 'cancelled' or code not in RETRYABLE_READ_FAILURES or query.get('attempts', 1) >= MAX_READ_ATTEMPTS:
                        raise CliError(code or 'JOB_TERMINAL')
                    query.setdefault('previousJobs', []).append({'jobId': query['jobId'], 'state': status['state'], 'failureCode': code})
                    query.pop('jobId')
            if not query.get('jobId'):
                if query.get('attempts', 0) >= MAX_READ_ATTEMPTS:
                    raise CliError('READ_ATTEMPTS_EXHAUSTED')
                if self.count >= self.state['context']['queryLimit']:
                    raise CliError('QUERY_LIMIT_REACHED')
                # Reserve before sending: an uncertain response still consumes a read attempt.
                self.count += 1
                self.state['submittedQueries'] = self.count
                query['attempts'] = query.get('attempts', 0) + 1
                self.save()
                submitted = cli(self.args.cli, ['jobs', 'submit', operation, '--version', contract['operationVersion'],
                                    '--params', stable(arguments), '--profile', self.args.profile])
                query['jobId'] = submitted['jobId']
                query['state'] = submitted['state']
                query.pop('failureCode', None)
                self.save()
            waited = cli(self.args.cli, ['jobs', 'wait', query['jobId'], '--timeout', str(JOB_WAIT_SECONDS), '--profile', self.args.profile])
            result = waited if waited.get('items') is not None else cli(self.args.cli, ['jobs', 'result', query['jobId'], '--profile', self.args.profile])
            if result.get('state') != 'succeeded' or result.get('complete') is not True or not isinstance(result.get('items'), list):
                raise CliError('PARTIAL_RESULT')
            meta = result.get('_meta', {})
            if query['executionLocation'] == 'local' and result.get('executionLocation') != 'local':
                raise CliError('CONTRACT_CHANGED')
            if meta.get('contractDigest') != contract['contractDigest'] or meta.get('contractVersion') != contract['operationVersion']:
                query['observedContract'] = {key: meta.get(key) for key in
                                             ('operationId', 'contractVersion', 'contractDigest', 'traceId')}
                raise CliError('CONTRACT_CHANGED')
            rows = result['items']
            atomic(result_file, rows)
            query.update(state='succeeded', sha256=hashlib.sha256(result_file.read_bytes()).hexdigest(), file=str(result_file),
                         acquiredAt=datetime.now(timezone.utc).isoformat())
            source = {'operationId': operation, 'operationVersion': contract['operationVersion'], 'contractDigest': contract['contractDigest'],
                      'arguments': arguments, 'jobId': query['jobId'], 'file': str(result_file), 'sha256': query['sha256'], 'acquiredAt': query['acquiredAt'],
                      'executionLocation': query['executionLocation'], 'readStartedAt': meta.get('readStartedAt'),
                      'readFinishedAt': meta.get('asOf'), 'snapshotConsistent': meta.get('snapshotConsistent', False), 'taskView': result.get('taskView')}
            self.state['sources'] = [row for row in self.state['sources'] if row.get('jobId') != query['jobId']] + [source]
            self.save()
            return rows
        except CliError as error:
            if error.code == 'PARTIAL_RESULT' and query.get('jobId'):
                try:
                    status = cli(self.args.cli, ['jobs', 'status', query['jobId'], '--profile', self.args.profile])
                    query['remoteState'] = status.get('state')
                    if status.get('state') in ('failed', 'cancelled'):
                        query['state'] = status['state']
                        error = CliError(status.get('failureCode') or 'JOB_TERMINAL')
                except CliError:
                    pass
            if error.code == 'RESULT_EXPIRED' and query.get('jobId') and query.get('attempts', 1) < MAX_READ_ATTEMPTS:
                query.setdefault('previousJobs', []).append({'jobId': query.pop('jobId'), 'state': 'succeeded',
                                                             'failureCode': error.code})
                query['state'] = 'new'
                self.save()
                return self.get(kind, arguments, scope_check=scope_check)
            query['failureCode'] = error.code
            self.issues.append(f'{kind}未完成：{error.code}，原任务{query.get("jobId", "尚未提交")}')
            self.save()
            return None


def extend(rows, values):
    if values is not None:
        rows.extend(values)
        return True
    return False


def run_queries(args, run_directory, state, record):
    queries = Queries(args, run_directory, state, record)
    request = state['context']
    brand = {'brandId': request['brandId']}
    shops = queries.current_shops(brand)
    if shops is None:
        state['state'] = 'partial'; queries.save()
        return {'state': 'partial', 'runId': state['runId'],
                'summary': '本人门店范围未读取完成，已保留任务记录。',
                'runDirectory': str(run_directory), 'issues': queries.issues}
    partition = queries.get('partitionTree', brand)
    region_by_id = partition_regions(partition or [])
    for shop in shops:
        matches = region_by_id.get(str(shop.get('id')), set())
        if len(matches) == 1:
            shop['region'] = next(iter(matches))
            shop['regionSource'] = '原系统当前分区树，按门店编号核对'
        elif len(matches) > 1:
            shop['region'] = None
            queries.issues.append(f'门店{shop["shopCode"]}当前大区对应关系冲突')
    allowed = {row['shopCode'] for row in shops}
    selected = set(request['shopCodes']) if request['shopCodes'] else allowed
    if request.get('regions'):
        regions = set(request['regions'])
        known = {row.get('region') for row in shops}
        if not regions <= known:
            raise ValueError('大区名称未匹配，需核对后继续原任务。')
        selected &= {row['shopCode'] for row in shops if row.get('region') in regions}
    if not selected or not selected <= allowed:
        raise ValueError('没有匹配到本人允许查看的门店。')
    request['authorizedShopCodes'] = sorted(allowed)
    request['shopCodes'] = sorted(selected)
    data = {'shops': [row for row in shops if row['shopCode'] in selected], 'indicators': [], 'orders': [], 'revenue': [],
            'dailyInventory': [], 'items': [], 'groups': [], 'classifications': [], 'freshConfig': [], 'inventoryHistory': [], 'dishConsume': []}
    main_complete = True
    for segment in request['segments']:
        for shop in sorted(selected):
            arguments = {**brand, **segment, 'shopCodes': [shop]}
            for kind in ('orders', 'indicators', 'revenue', 'dailyInventory'):
                if not extend(data[kind], queries.get(kind, arguments)):
                    main_complete = False
    if not extend(data['items'], queries.get('items', brand)):
        main_complete = False
    extend(data['classifications'], queries.get('classifications', brand))
    extend(data['freshConfig'], queries.get('freshConfig', brand))
    # Only query groups that appear in the analysis. The full items catalog still supplies the member metadata.
    used = {str(row.get('itemCode')) for row in data['orders'] + data['indicators'] + data['dailyInventory']}
    for item in data['items']:
        code = str(item.get('code', item.get('itemCode')))
        if code not in used or item.get('id') is None:
            continue
        members = queries.get('groups', {**brand, 'itemId': item['id']})
        if members is not None:
            data['groups'].append({'itemCode': code, 'memberCodes': list(dict.fromkeys([code, *[str(row['code']) for row in members]])),
                                   'complete': True, 'source': '原接口完整同组读取'})
    start, end = day(request['startDate']), day(request['endDate'])
    # Fetch earlier ranges until each observed shop/item has 14 valid-revenue days, bounded by the explicit limit.
    cursor = start - timedelta(days=1)
    earliest = start - timedelta(days=LOOKBACK_LIMIT_DAYS)
    for _ in range((LOOKBACK_LIMIT_DAYS + SEGMENT_DAYS - 1) // SEGMENT_DAYS):
        needs = baseline_needs(data['dailyInventory'], used, selected, start)
        if not needs:
            break
        left = max(earliest, cursor - timedelta(days=SEGMENT_DAYS - 1))
        for shop in sorted({key[0] for key in needs}):
            values = queries.get('dailyInventory', {**brand, 'shopCodes': [shop], 'startDate': left.isoformat(), 'endDate': cursor.isoformat()})
            extend(data['dailyInventory'], values)
        cursor = left - timedelta(days=1)
        if queries.count >= request['queryLimit'] or cursor < earliest:
            break
    if baseline_needs(data['dailyInventory'], used, selected, start):
        queries.issues.append('此前14个有效营业额日资料不足，报告保留实际样本数')
    warehouses = {}
    group_ids = set(request.get('warehouseGroups', []))
    tree = queries.get('shopTeamTree', brand)
    group_ids.update(team_ids(tree or []))
    for group in sorted(group_ids):
        rows = queries.get('shopTeams', {**brand, 'groupId': group})
        for row in rows or []:
            if row.get('shopCode') in selected and isinstance(row.get('warehouseId'), int) and row['warehouseId'] > 0:
                warehouses.setdefault(row['shopCode'], set()).add(row['warehouseId'])
    for shop, ids in warehouses.items():
        for warehouse in ids:
            checked = queries.get('warehouse', {**brand, 'shopCodes': [shop], 'warehouseId': warehouse})
            if checked is None:
                continue
            for segment in request['segments']:
                extend(data['inventoryHistory'], queries.get('inventoryHistory', {**brand, 'shopCodes': [shop], 'warehouseId': warehouse, **segment}))
    if not warehouses:
        queries.issues.append('尚未取得已知仓库范围；可继续提供门店组编号或核对已有仓库，不能据此认定缺盘')
    elif set(warehouses) != selected:
        queries.issues.append('部分目标门店未取得已有仓库；门店组只返回首个仓库，全部仓库覆盖未核实')
    else:
        queries.issues.append('门店组只返回首个已有仓库，全部仓库覆盖未核实，不能据此判断缺盘')
    bundle = {'schemaVersion': RESULT_SCHEMA, 'sourceKind': SOURCE_LOCAL if request.get('environment') == 'local' else SOURCE_LIVE, 'context': request, 'data': data,
              'sources': state['sources'], 'coverage': {'complete': main_complete, 'issues': queries.issues,
              'completedQueries': [row for row in state['queries'].values() if row['state'] == 'succeeded' and row.get('availability') != 'unavailable'],
              'unfinishedQueries': [row for row in state['queries'].values() if row['state'] != 'succeeded' or row.get('availability') == 'unavailable']}}
    preliminary = analyze(bundle)
    target_shops = {row['shopCode'] for row in preliminary['focusTargets']}
    for anomaly in preliminary['dailyComparisons']:
        if not anomaly['anomaly'] or anomaly['shopCode'] not in target_shops:
            continue
        for value_date in [anomaly['date'], *anomaly['baselineDates']]:
            rows = queries.get('dishConsume', {**brand, 'shopCodes': [anomaly['shopCode']], 'itemCode': anomaly['itemCode'], 'date': value_date})
            if rows is not None:
                data['dishConsume'].append({'shopCode': anomaly['shopCode'], 'itemCode': anomaly['itemCode'], 'date': value_date, 'items': rows, 'complete': True})
    bundle['sources'] = state['sources']
    bundle['coverage']['issues'] = list(dict.fromkeys(queries.issues))
    bundle['coverage']['completedQueries'] = [row for row in state['queries'].values() if row['state'] == 'succeeded' and row.get('availability') != 'unavailable']
    bundle['coverage']['unfinishedQueries'] = [row for row in state['queries'].values() if row['state'] != 'succeeded' or row.get('availability') == 'unavailable']
    bundle['coverage']['complete'] = main_complete and not bundle['coverage']['unfinishedQueries']
    atomic(run_directory / BUNDLE_FILE, bundle)
    output = write_result(bundle, run_directory / REPORT_DIRECTORY)
    state.update(state='completed' if main_complete and not bundle['coverage']['unfinishedQueries'] else 'partial',
                 completedStages=['范围核对', '受控读取已完成部分', '计算', '报告'], output=output)
    queries.save()
    return {**output, 'runId': state['runId'], 'state': state['state'], 'continueCommand':
            f'{sys.executable} {ROOT / "scripts/run.py"} --resume {run_directory} --profile {args.profile}'}


def team_ids(nodes):
    values = set()
    for node in nodes:
        children = node.get('children') or []
        if node.get('id') is not None:
            values.add(str(node['id']))
        values.update(team_ids(children))
    return values


def partition_regions(nodes, path=()):
    result = {}
    for node in nodes:
        here = (*path, node.get('name'))
        # The source tree contains partition roots, not a brand wrapper. Prefer an explicit
        # region name; keep ambiguous paths unassigned rather than mislabeling a brand.
        regions = [name for name in here if isinstance(name, str) and name.endswith('大区')]
        region = regions[0] if len(set(regions)) == 1 else (here[0] if len(here) == 1 else None)
        for shop in node.get('shopInfoList') or []:
            if shop.get('id') is not None and region:
                result.setdefault(str(shop['id']), set()).add(region)
        for key, names in partition_regions(node.get('children') or [], here).items():
            result.setdefault(key, set()).update(names)
    return result


def baseline_needs(records, used, shops, start):
    from analyze import number, positive
    seen = {}
    for row in records:
        when = day(row.get('statisticDate'))
        if not when or when >= start or not positive(row.get('actualRevenue')) or number(row.get('theoryConsume')) is None:
            continue
        key = (row.get('shopCode'), str(row.get('itemCode')))
        seen.setdefault(key, set()).add(when)
    observed = {(row.get('shopCode'), str(row.get('itemCode'))) for row in records if row.get('shopCode') in shops
                and str(row.get('itemCode')) in used}
    return [key for key in observed if len(seen.get(key, set())) < BASELINE_VALID_DAYS]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', default=shutil.which('dt-cli') or 'dt-cli')
    parser.add_argument('--profile', required=True)
    parser.add_argument('--client', choices=('Codex', 'WorkBuddy'), default='Codex')
    parser.add_argument('--skill-directory')
    parser.add_argument('--brand', type=int)
    parser.add_argument('--period', choices=('上周', '近7天'))
    parser.add_argument('--start'); parser.add_argument('--end')
    parser.add_argument('--shop', action='append', default=[])
    parser.add_argument('--region', action='append', default=[])
    parser.add_argument('--warehouse-group', action='append', default=[])
    parser.add_argument('--query-limit', type=int)
    parser.add_argument('--resume', type=Path)
    parser.add_argument('--initialize', action='store_true', help=argparse.SUPPRESS)
    parser.add_argument('--confirm-read', action='store_true', help='当前客户端确实读取本任务资料后记录；不能代替真实业务验收')
    args = parser.parse_args()
    if args.skill_directory is None:
        args.skill_directory = str(Path.home() / CLIENT_DIRECTORIES[args.client] / SKILL_NAME)
    try:
        if args.resume is None:
            if args.initialize:
                raise ValueError('任务初始化需要已准备的运行目录。')
            parser.exit(new_task(args))
        root = args.resume.expanduser().resolve()
        record = load(root / RUN_RECORD)
        resources = verify_resources(root, record)
        if Path(__file__).resolve() != resources / 'scripts/run.py':
            parser.exit(cli_script(args, resources / 'scripts/run.py', ['--resume', root, '--profile', args.profile,
                        *(['--initialize', *request_arguments(args)] if args.initialize else []),
                        *(['--confirm-read'] if args.confirm_read else [])]))
        state = initialize_task(args, root, record) if args.initialize else load(root / STATE_FILE)
        if args.profile != state['profile']:
            raise ValueError('恢复使用原 profile。')
        confirmed = state.setdefault('clientReads', {})
        if args.confirm_read:
            confirmed[args.client] = {'skillVersion': state['skillVersion'],
                                     'calculationVersion': state['calculationVersion'],
                                     'entryFile': str(resources / ENTRY_FILE),
                                     'confirmedAt': datetime.now(timezone.utc).isoformat()}
            state['clientReadConfirmed'] = True
            atomic(root / STATE_FILE, state)
        if args.client not in confirmed:
            print(json.dumps({'runId': state['runId'], 'resourcesDirectory': str(resources), 'entryFile': str(resources / ENTRY_FILE),
                 'state': 'needs-client-read', 'message': '请当前AI工具读取本次保存的SKILL.md，确认后使用--confirm-read继续。'}, ensure_ascii=False))
            return
        check_identity(args, state, record)
        print(json.dumps(run_queries(args, root, state, record), ensure_ascii=False, default=str))
    except (ValueError, OSError, CliError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f'任务未完成：{error}\n')


if __name__ == '__main__':
    main()
