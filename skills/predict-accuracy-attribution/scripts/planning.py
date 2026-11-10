"""原查询范围与营业额基准规划；独立于 CLI 执行器。"""
from analyze import day
from rules import BASELINE_VALID_DAYS

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
