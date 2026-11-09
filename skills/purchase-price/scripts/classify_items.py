"""按原料肉或原虾范围整理候选；只离线分类，不登录、不查询价格。"""

import argparse
import json
import sys
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import List, Optional, Tuple


@dataclass
class ItemCandidate:
    # 同系统真实品项ID，正整数。
    id: int
    # 完整业务编码，保留前导零。
    code: str
    # 原品项名称，不自行改写。
    name: str
    # 原规格；接口未提供时为空。
    specification: Optional[str]
    # 原系统返回的类目名称；未知时为空，不借用其他系统类目。
    category_name: Optional[str]


@dataclass
class FamilyRules:
    # 本次原料食材范围。
    family: str
    # 可证明畜种或原虾的名称线索，仍须排除产品形态。
    species_terms: Tuple[str, ...]
    # 可证明原料或常规部位的名称线索。
    raw_terms: Tuple[str, ...]
    # 明确其他食材或物料的名称，不按单字误归类。
    unrelated_terms: Tuple[str, ...]
    # 默认原料肉范围之外的内脏、骨料及副产品。
    byproduct_terms: Tuple[str, ...]


@dataclass
class ItemDecision:
    # 本次保留的真实候选。
    item: ItemCandidate
    # 纳入、排除、待核实；一个ID只能属于一类。
    decision: str
    # 分类依据说明，供内部复核。
    reason: str


@dataclass
class ReviewDecision:
    # 待复核的真实候选ID。
    id: int
    # 经证据核对后的纳入、排除或待核实。
    decision: str
    # 对名称、规格、形态或来源冲突的实际核对结论。
    reason: str
    # 依据的原始文件或用户明确范围；不能为空。
    source: str


RULES = {
    '牛肉': FamilyRules('牛肉', ('牛肉', '牛腩', '牛腱', '牛肋', '牛肩', '牛胸', '牛霖', '牛柳',
        '牛排', '牛扒', '牛腿', '牛脸肉', '肥牛', '牛前', '牛后', '牛翼', '牛股', '牛吊龙', '牛上脑',
        '牛坑腩', '牛腹', '牛小排', '牛板腱', '牛板键', '牛骨', '牛筋', '牛油', '牛体脂'),
        ('牛肉', '牛腩', '牛腱', '牛肋', '牛肩', '牛胸', '牛霖', '牛柳', '牛排', '牛扒', '牛腿',
         '牛脸肉', '肥牛', '西冷', '菲力', '眼肉', '牛上脑', '牛吊龙', '牛坑腩', '牛小排',
         '牛前', '牛后', '牛板腱', '牛板键', '肉'),
        ('牛奶', '牛乳', '乳酪', '奶酪', '牛肝菌', '牛蛙', '红牛', '牛皮纸', '牛肉味', '牛油果', '牛油生菜', '牛栏山'),
        ('牛肚', '牛舌', '牛肠', '牛肝', '牛心', '牛肺', '牛杂', '牛百叶', '牛鞭', '牛蹄筋',
         '牛皮', '牛骨', '牛棒骨', '牛筒骨', '牛胫骨', '大骨头', '牛白叶', '牛白千层肚', '千层肚',
         '牛板肚', '牛筋', '牛孖筋', '牛黄喉', '牛小肠', '牛大骨', '牛脊骨', '牛龙骨', '牛颈骨',
         '牛油', '牛肥油', '牛腰油', '牛脂', '牛体脂', '牛肉肥油', '牛肉碎脂')),
    '猪肉': FamilyRules('猪肉', ('猪肉', '猪腿', '猪肩', '猪里脊', '猪五花', '猪排', '猪骨', '猪肝', '猪肚'),
        ('猪肉', '五花', '里脊', '猪腿', '猪肩', '梅头', '梅花肉', '前腿', '后腿', '二刀肉',
         '排骨', '肉片', '肉丝', '肉馅', '肉块'),
        ('猪笼', '猪模具', '猪肉味'),
        ('猪肝', '猪心', '猪肺', '猪肚', '猪肠', '猪杂', '猪皮', '猪油', '猪骨', '猪棒骨', '猪筒骨')),
    '鸡肉': FamilyRules('鸡肉', ('鸡肉', '鸡胸', '鸡腿', '鸡翅', '鸡丁', '鸡块', '整鸡', '三黄鸡', '土鸡', '仔鸡'),
        ('鸡肉', '鸡胸', '鸡腿', '鸡翅', '鸡丁', '鸡块', '整鸡', '三黄鸡', '土鸡', '仔鸡', '肉片', '肉丝'),
        ('鸡蛋', '鸡精', '鸡汁', '鸡粉', '鸡油菌'),
        ('鸡肝', '鸡心', '鸡肠', '鸡杂', '鸡爪', '鸡架', '鸡骨')),
    '虾': FamilyRules('虾', ('虾',),
        ('虾',), ('虾模具', '虾餐具'),
        ('虾米', '虾皮', '虾干')),
}
FORMED = ('肉丸', '肉饺', '肉包', '肉饼', '肉松', '肉干', '罐头', '香肠', '火腿', '牛肉汁', '牛肉粉',
          '虾滑', '虾酱', '虾油', '虾抽', '虾饺', '虾饼', '虾排', '汤包', '酱料', '调料', '调味料',
          '肉酱', '料粉', '肉膏', '原汤', '浓汤', '料包', '味汁', '风味', '桶面', '肉面', '肉酥', '自热',
          '调味酱', '调味粉', '专用腌', '专用嫩滑', '腌粉', '蔬菜组合包', '豆豉油辣椒')
COOKED = ('红烧', '酱香', '酱牛肉', '卤', '香拌', '熟', '麻辣', '照烧', '咖喱',
          '黑椒', '五香', '清汤', '盐焗', '白灼', '油炸', '预制菜', '香辣', '粉蒸', '金汤',
          '焖', '炖', '炒', '焯水', '白切', '烧牛肉', '烧牛腩')
UNCLEAR = ('调理', '原味', '腌', '调味', '胸口油', '水晶', 'QQ虾', '蝴蝶虾', '虾串', '组合菜', '牛头肉')
MATERIAL = ('餐具', '模具', '一次性用品', '包装材料', '工具', '器具', '厨具', '清洁', '工服', '服装',
            '低值易耗', '包装物', '办公用品', '员工餐', '方便速食', '标签牌')
NON_MEAT = ('调料', '香料', '汁酱', '油类', '乳品', '乳制品', '叶菜', '根果', '其它菜', '加工菜',
            '米类', '面类', '酒类', '饮料', '菌菇')
MEAT_FAMILIES = ('牛肉', '猪肉', '鸡肉', '羊肉', '虾')


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('JSON包含重复字段。')
        result[key] = value
    return result


def read_json(path):
    with Path(path).open('rb') as stream:
        raw = stream.read(16777217)
    if len(raw) > 16777216:
        raise ValueError('候选文件超过读取上限。')
    return json.loads(raw.decode('utf-8-sig'), object_pairs_hook=unique_object)


def read_candidates(path):
    value = read_json(path)
    # 输入是已成功取得的业务正文/list，不把CLI信封误当业务列表。
    rows = value.get('list') if isinstance(value, dict) else value
    if not isinstance(rows, list):
        raise ValueError('候选须为业务正文的list或品项数组。')
    result = []
    for item in rows:
        if not isinstance(item, dict) or type(item.get('id')) is not int or item['id'] < 1:
            raise ValueError('候选缺少真实正整数品项ID。')
        if any(not isinstance(item.get(k), str) or not item[k].strip() for k in ('code', 'name')):
            raise ValueError('候选须保留完整字符串编码与名称。')
        for key in ('specification', 'itemCategoryName'):
            if item.get(key) is not None and not isinstance(item[key], str):
                raise ValueError('候选规格或类目名称类型错误。')
        result.append(ItemCandidate(item['id'], item['code'], item['name'],
                                    item.get('specification'), item.get('itemCategoryName')))
    return result


def contains(text, terms):
    return any(term in text for term in terms)


def decide(item, rules):
    name, category = item.name, item.category_name or ''
    if contains(name, rules.unrelated_terms) or contains(category, MATERIAL) or contains(name, MATERIAL):
        return ItemDecision(item, '排除', '名称或类目明确属于其他食材或物料。')
    direct_species = contains(name, rules.species_terms)
    category_species = rules.family in category or rules.family == '鸡肉' and '鸡类' in category
    other_category = any(term in category for term in MEAT_FAMILIES if term != rules.family)
    species = direct_species or category_species
    if not species:
        if other_category:
            return ItemDecision(item, '排除', '实际类目明确属于其他畜种。')
        if contains(name, rules.raw_terms) and (not category or contains(category, ('肉', '水产', '海鲜'))):
            return ItemDecision(item, '待核实', '部位名称尚不足以确认本次畜种。')
        return ItemDecision(item, '排除', '名称和类目没有支持本次原料食材的证据。')
    if contains(name, rules.byproduct_terms):
        return ItemDecision(item, '排除', '属于默认原料肉范围外的内脏、骨料或副产品。')
    if contains(name, FORMED) or contains(name, COOKED):
        return ItemDecision(item, '排除', '名称明确为成型制品、熟制品或调味制品。')
    if contains(category, NON_MEAT) or other_category:
        return ItemDecision(item, '待核实', '名称食材线索与实际类目不一致，尚不能确认归属。')
    if contains(name, UNCLEAR):
        return ItemDecision(item, '待核实', '加工形态或业务归属需要名称、规格以外的进一步证据。')
    if not contains(name, rules.raw_terms):
        return ItemDecision(item, '待核实', '畜种已确认，名称尚不足以确认原料或常规分切形态。')
    # 加工牛肉类也可能只是切条、切块；去骨和带骨不是加工制品证据。
    return ItemDecision(item, '纳入', '名称和畜种线索支持原料或常规分切形态。')


def read_reviews(path):
    if path is None:
        return []
    value = read_json(path)
    if not isinstance(value, dict) or not isinstance(value.get('decisions'), list):
        raise ValueError('复核文件须有具名decisions数组。')
    result = []
    for row in value['decisions']:
        if not isinstance(row, dict) or type(row.get('id')) is not int or row['id'] < 1:
            raise ValueError('复核记录须有真实品项ID。')
        if row.get('decision') not in ('纳入', '排除', '待核实') or any(
                not isinstance(row.get(k), str) or not row[k].strip() for k in ('reason', 'source')):
            raise ValueError('复核须有分类、实际依据和来源。')
        result.append(ReviewDecision(row['id'], row['decision'], row['reason'], row['source']))
    if len({row.id for row in result}) != len(result):
        raise ValueError('同一品项出现重复或矛盾复核。')
    return result


def classify(candidates, family, reviews=()):
    unique, conflicts = {}, set()
    for item in candidates:
        previous = unique.get(item.id)
        if previous is not None and previous != item:
            conflicts.add(item.id)
        else:
            unique[item.id] = item
    decisions = {key: decide(item, RULES[family]) for key, item in unique.items()}
    for key in conflicts:
        decisions[key] = ItemDecision(unique[key], '待核实', '同ID的名称、编码、规格或类目存在冲突。')
    for review in reviews:
        if review.id not in decisions:
            raise ValueError('复核ID不在本次真实候选内。')
        if review.id in conflicts and review.decision == '纳入':
            raise ValueError('同ID来源冲突须先取得一致数据，不能直接纳入。')
        decisions[review.id] = ItemDecision(unique[review.id], review.decision,
                                           review.reason + ' 来源：' + review.source)
    return sorted(decisions.values(), key=lambda row: row.item.id)


def selection_wire(decisions, family, source_count):
    included = [row for row in decisions if row.decision == '纳入']
    pending = [row for row in decisions if row.decision == '待核实']
    excluded = [row for row in decisions if row.decision == '排除']
    sets = [{row.item.id for row in group} for group in (included, pending, excluded)]
    if any(a & b for index, a in enumerate(sets) for b in sets[index + 1:]):
        raise ValueError('分类集合重叠，不能生成查价集合。')
    if sum(map(len, sets)) != len(decisions):
        raise ValueError('候选分类未完整覆盖。')
    # 此处仅序列化具名候选与分类结果，不生成完整采购价请求。
    return {'family': family, 'itemIds': sorted(sets[0]),
        'included': [asdict(row) for row in included], 'pending': [asdict(row) for row in pending],
        'excluded': [asdict(row) for row in excluded],
        'counts': {'raw': source_count, 'unique': len(decisions), 'included': len(included),
                   'pending': len(pending), 'excluded': len(excluded)},
        'categoryCoverageComplete': False}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--family', choices=tuple(RULES), required=True, help='用户要求的原料食材类别')
    parser.add_argument('--input', action='append', required=True, help='已成功取得的业务正文；每页一个文件')
    parser.add_argument('--review-file', help='只对有实际依据的边界项作具名复核')
    parser.add_argument('--output', required=True, help='新的分类结果文件')
    args = parser.parse_args(argv)
    try:
        output = Path(args.output).resolve()
        if output.exists():
            raise ValueError('分类输出已存在。')
        candidates = []
        for path in args.input:
            candidates.extend(read_candidates(path))
        if len(candidates) > 10000:
            raise ValueError('候选数量超过工具上限；继续遵守更窄的任务预算。')
        decisions = classify(candidates, args.family, read_reviews(args.review_file))
        result = selection_wire(decisions, args.family, len(candidates))
        with output.open('x', encoding='utf-8', newline='\n') as stream:
            stream.write(json.dumps(result, ensure_ascii=False, allow_nan=False, indent=2) + '\n')
        print(json.dumps({'ok': True, 'data': {'selectionFile': str(output),
                         'counts': result['counts']}}, ensure_ascii=False))
        return 0
    except (OSError, ValueError, UnicodeError, TypeError):
        print(json.dumps({'ok': False, 'error': {'code': 'ITEM_CLASSIFICATION_FAILED',
            'message': '候选分类未完成，请核对真实ID、编码、来源、复核依据及新的输出文件。'}}, ensure_ascii=False))
        return 2


if __name__ == '__main__':
    sys.exit(main())
