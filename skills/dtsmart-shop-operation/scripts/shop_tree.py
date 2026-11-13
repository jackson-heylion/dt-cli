"""从餐饮军师当前员工可见的组织树解析门店。"""

import re
from dataclasses import dataclass
from typing import Optional

from operations import ENDPOINTS
from platform_read import PlatformError

SHOP_TREE = "shop-tree"
SHOP_TREE_TYPE = "GROUP"
VALID_SHOP_CODE = re.compile(r"^[A-Za-z0-9_-]+$")
VALID_BRAND_NO = re.compile(r"^[1-9][0-9]{0,9}$")


def shop_tree_body(brand_no):
    if brand_no is None:
        return {"type": SHOP_TREE_TYPE}
    if not VALID_BRAND_NO.fullmatch(brand_no):
        raise SystemExit("[参数错误] --brand-no 需要一个有效的餐饮军师品牌编码。")
    return {"type": SHOP_TREE_TYPE, "brandNo": brand_no}


@dataclass(frozen=True)
class Shop:
    # 餐饮军师门店主数据 ID。
    shopId: str
    # 经营接口使用的组织编码；档案未维护时为空。
    shopCode: Optional[str]
    # 门店展示全名。
    shopName: str
    # 从根节点到所在分组的名称路径。
    groupPath: str


def split_list(value):
    return [part.strip() for part in (value or "").split(",") if part.strip()]


def flatten_tree(nodes, groups=()):
    if not isinstance(nodes, list):
        raise PlatformError("SHOP_LOOKUP_INVALID", "门店树结果不是节点列表。")
    for node in nodes:
        if (not isinstance(node, dict) or not isinstance(node.get("name"), str)
                or type(node.get("leaf")) is not bool):
            raise PlatformError("SHOP_LOOKUP_INVALID", "门店树节点缺少名称或叶节点标识。")
        name = node["name"]
        if node["leaf"]:
            shop_id = node.get("id")
            if not isinstance(shop_id, str) or not shop_id:
                raise PlatformError("SHOP_LOOKUP_INVALID", "门店叶节点缺少主数据 ID。")
            shop_code = node.get("shopCode")
            if shop_code is not None and not isinstance(shop_code, str):
                raise PlatformError("SHOP_LOOKUP_INVALID", "门店组织编码必须为字符串。")
            yield Shop(shop_id, shop_code, name, " / ".join(groups))
        elif node.get("children") is not None:
            yield from flatten_tree(node["children"], (*groups, name))


def get_shops(client, body):
    result = client.read(SHOP_TREE, ENDPOINTS[SHOP_TREE], body, 1)
    return list(flatten_tree(result.data))


def resolve_codes(rows, names, explicit_codes):
    ids_by_code = {}
    for row in rows:
        ids_by_code.setdefault(row.shopCode, set()).add(row.shopId)
    selected = []
    for name in names:
        matches = {(row.shopId, row.shopCode) for row in rows if row.shopName == name
                   and (not explicit_codes or row.shopCode in explicit_codes)}
        if not matches:
            raise PlatformError("SHOP_NOT_FOUND", "本人门店树中没有符合所选名称和编码的门店。")
        if len(matches) != 1:
            raise PlatformError("SHOP_AMBIGUOUS", "本人门店树有重名门店，请先按全名查树，再用已核对的编码。")
        _, code = matches.pop()
        if not isinstance(code, str) or not VALID_SHOP_CODE.fullmatch(code) or len(ids_by_code[code]) != 1:
            raise PlatformError("SHOP_AMBIGUOUS", "门店编码缺失或对应多个门店，无法用于经营查询。")
        selected.append(code)
    return list(dict.fromkeys(selected))
