"""验证真实畜种、常规分切与边界项互斥，避免用制品替代原料。"""

import importlib.util
import pathlib
import sys
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[2] / 'skills/purchase-price/scripts/classify_items.py'
SPEC = importlib.util.spec_from_file_location('dt_item_classification_tested', SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


def candidate(number, name, category='牛肉类', specification='抄码'):
    return MODULE.ItemCandidate(number, f'001{number}', name, specification, category)


class ItemClassificationTest(unittest.TestCase):
    def test_deboned_and_plain_cut_meat_remain_raw_even_in_processed_category(self):
        rows = [candidate(1, '冷冻去骨牛腩'), candidate(2, '带骨牛肋排'),
                candidate(3, '牛腩切块', '加工牛肉类'), candidate(4, '牛腩（切条）', '加工牛肉类')]
        result = MODULE.selection_wire(MODULE.classify(rows, '牛肉'), '牛肉', len(rows))
        self.assertEqual(result['itemIds'], [1, 2, 3, 4])

    def test_unrelated_cow_names_and_formed_food_do_not_pollute_raw_meat(self):
        rows = [candidate(1, '全脂牛奶', '调料类'), candidate(2, '牛肝菌', '菌菇类'),
                candidate(3, '牛蛙', '蛙类'), candidate(4, '红牛酱料包', '香料'),
                candidate(5, '牛肉丸'), candidate(6, '牛肉饺'), candidate(7, '红烧牛肉'),
                candidate(8, '猪肉片（汝子牛）', '猪肉类'), candidate(9, '牛肉标签牌', '低值易耗类'),
                candidate(10, '牛肉膏', '调料类'), candidate(11, '牛油果', '根果类'),
                candidate(12, '小炒黄牛肉组合餐', '员工餐小类')]
        result = MODULE.selection_wire(MODULE.classify(rows, '牛肉'), '牛肉', len(rows))
        self.assertEqual(result['itemIds'], [])
        self.assertEqual(len(result['excluded']), len(rows))

    def test_raw_name_with_inconsistent_category_is_pending_instead_of_silently_included(self):
        rows = [candidate(1, '牛肋条腹肉', '调料类'), candidate(2, '佳品肥牛', '羊肉类')]
        result = MODULE.selection_wire(MODULE.classify(rows, '牛肉'), '牛肉', len(rows))
        self.assertEqual(result['itemIds'], [])
        self.assertEqual(len(result['pending']), 2)

    def test_ambiguous_flavor_and_conflicting_rows_never_enter_price_ids(self):
        rows = [candidate(1, '原味牛脸肉'), candidate(2, '原味牛肉'), candidate(3, '去骨牛肉（调理牛肉）'),
                candidate(4, '牛腩'), candidate(4, '牛奶', '调料类')]
        result = MODULE.selection_wire(MODULE.classify(rows, '牛肉'), '牛肉', len(rows))
        self.assertEqual(result['itemIds'], [])
        self.assertEqual({r['item']['id'] for r in result['pending']}, {1, 2, 3, 4})
        self.assertEqual(result['counts']['unique'], 4)

    def test_marinated_meat_and_braised_dishes_are_not_treated_as_plain_cuts(self):
        rows = [candidate(1, '腌牛腩切块', '加工牛肉类'), candidate(2, '番茄焖牛肉', '加工牛肉类'),
                candidate(3, '牛肉肥油', '加工牛肉类'), candidate(4, '焯水牛腩块', '加工牛肉类')]
        result = MODULE.selection_wire(MODULE.classify(rows, '牛肉'), '牛肉', len(rows))
        self.assertEqual(result['itemIds'], [])
        self.assertEqual(result['pending'][0]['item']['id'], 1)

    def test_every_unique_id_has_one_classification_and_full_code_is_kept(self):
        rows = [candidate(1, '牛腩'), candidate(1, '牛腩'), candidate(2, '原味牛肉'), candidate(3, '牛肚')]
        result = MODULE.selection_wire(MODULE.classify(rows, '牛肉'), '牛肉', len(rows))
        groups = [{r['item']['id'] for r in result[name]} for name in ('included', 'pending', 'excluded')]
        self.assertEqual(set.union(*groups), {1, 2, 3})
        self.assertFalse(any(a & b for i, a in enumerate(groups) for b in groups[i + 1:]))
        self.assertEqual(result['included'][0]['item']['code'], '0011')
        self.assertFalse(result['categoryCoverageComplete'])

    def test_pork_meatball_and_shrimp_paste_cannot_replace_raw_results(self):
        pork = [candidate(1, '猪肉丸', '加工猪肉类'), candidate(2, '猪五花肉', '猪肉类')]
        shrimp = [candidate(3, '虾滑', '虾类'), candidate(4, '冻青虾仁', '虾类'),
                  candidate(5, '腌制虾仁', '虾类')]
        self.assertEqual(MODULE.selection_wire(MODULE.classify(pork, '猪肉'), '猪肉', 2)['itemIds'], [2])
        result = MODULE.selection_wire(MODULE.classify(shrimp, '虾'), '虾', 3)
        self.assertEqual(result['itemIds'], [4])
        self.assertEqual(result['pending'][0]['item']['id'], 5)

    def test_conflicting_source_cannot_be_forced_into_price_collection(self):
        rows = [candidate(1, '牛腩'), candidate(1, '牛奶', '调料类')]
        review = MODULE.ReviewDecision(1, '纳入', '复核', '原文件')
        with self.assertRaises(ValueError):
            MODULE.classify(rows, '牛肉', [review])


if __name__ == '__main__':
    unittest.main()
