#!/usr/bin/env python3
"""经开放平台读取餐饮军师经营数据。"""

import argparse
import json
import sys
from dataclasses import asdict
from datetime import datetime
from pathlib import Path

from operations import ALIASES, ENDPOINTS
from platform_read import SYSTEM, PlatformClient, PlatformError, ReadResult, alias
from result_output import project_rows, render_table, write_csv, write_json
from shop_tree import SHOP_TREE, VALID_SHOP_CODE, get_shops, resolve_codes, shop_tree_body, split_list

PAGE_SIZE_DEFAULT = 200
PREVIEW_LIMIT_DEFAULT = 200
MAX_PAGES_DEFAULT = 50


def parse_date(text, label):
    try:
        return datetime.strptime(text, "%Y-%m-%d").date()
    except (ValueError, TypeError):
        raise SystemExit(f"[参数错误] {label} 需为 yyyy-MM-dd，收到: {text!r}")


def build_body(spec, args, body):
    body = dict(body)

    if args.start:
        body["startDate"] = args.start
    if args.end:
        body["endDate"] = args.end

    named = {
        "timeCycle": args.time_cycle,
        "metric": args.metric,
        "channel": args.channel,
        "timeSlot": args.time_slot,
        "dimension": args.dimension,
        "granularity": args.granularity,
        "summaryType": args.summary_type,
        "baselineWeeks": args.baseline_weeks,
        "noteType": args.note_type,
    }
    for key, value in named.items():
        if value is not None:
            body[key] = value
    if args.metric_keyword:
        body["metricKeyword"] = args.metric_keyword
    if args.compare_type:
        body["compareType"] = args.compare_type
    if args.section_name:
        body["sectionName"] = args.section_name
    if args.brand_names:
        body["brandNameList"] = split_list(args.brand_names)
    if args.period_labels:
        body["periodLabelList"] = split_list(args.period_labels)
    if args.channel_list:
        body["channelList"] = split_list(args.channel_list)

    if spec["paged"] and (args.page is not None or args.size is not None):
        if args.page is not None:
            body["page"] = args.page
        if args.size is not None:
            body["size"] = args.size
    return body


def validate(cmd, spec, body, shop_codes, pending_lookup=False):
    if spec["shops"] != "never" and not shop_codes and not pending_lookup:
        raise SystemExit(
            f"[缺少门店] {cmd} 需要 shopCodeList：用 --shop-codes 直接给，"
            "或用 --shop-name 指定全名，让脚本从本人门店树解析。"
        )
    if spec.get("max_shops") and len(shop_codes) > spec["max_shops"]:
        raise SystemExit(f"[参数错误] {cmd} 的 shopCodeList 上限 {spec['max_shops']} 个门店，"
                         f"当前 {len(shop_codes)} 个，请收窄范围。")
    if spec.get("no_dates"):
        return
    if not body.get("startDate") or not body.get("endDate"):
        raise SystemExit(f"[缺少日期] {cmd} 需要 --start 与 --end（yyyy-MM-dd）")
    start = parse_date(body["startDate"], "startDate")
    end = parse_date(body["endDate"], "endDate")
    if start > end:
        raise SystemExit("[参数错误] --start 不能晚于 --end")
    span = (end - start).days + 1
    rule = spec.get("span92")
    limited = False
    if rule is True:
        limited = True
    elif isinstance(rule, tuple):
        field, values = rule
        current = body.get(field) or ("ALL" if field == "timeCycle" else None)
        limited = current in values
    if limited and span > 92:
        if rule is True:
            why = "本接口"
        else:
            why = f"{rule[0]}={body.get(rule[0]) or 'ALL'} 时"
        raise SystemExit(
            f"[跨度超限] {cmd} {why}最多查 92 天，当前 {span} 天。"
            "请缩短区间、改用月/年聚合粒度。"
        )


class Parser(argparse.ArgumentParser):
    def error(self, message):
        self.print_usage(sys.stderr)
        raise SystemExit(f"[参数错误] {message}")


def add_shop_selection(p):
    p.add_argument("--shop-codes", help="门店组织编码，逗号分隔")
    p.add_argument("--shop-name", help="门店全名，逗号分隔")
    p.add_argument("--brand-no", help="STG 查店时选择餐饮军师品牌编码；缺省沿用上游默认品牌")


def add_output(p):
    p.add_argument("--fields", help="只保留指定字段，逗号分隔")
    p.add_argument("--limit", type=int, default=PREVIEW_LIMIT_DEFAULT,
                   help=f"stdout 最多返回行数，默认 {PREVIEW_LIMIT_DEFAULT}（0=不限）")
    p.add_argument("--out", help="结果写入文件（.json 或 .csv）")
    p.add_argument("--format", choices=["json", "table"], default="json", help="stdout 输出格式")
    p.add_argument("--timeout", type=int, default=60, help="单次请求超时秒数，默认 60")
    p.add_argument("--dry-run", action="store_true", help="只打印请求体，不发请求")
    p.add_argument("--pretty", action="store_true", help="JSON 缩进输出")


def add_common(p):
    p.add_argument("--start", help="开始日期 yyyy-MM-dd（含）")
    p.add_argument("--end", help="结束日期 yyyy-MM-dd（含）")
    p.add_argument("--param", action="append", metavar="K=V",
                   help="补充契约字段，可重复；数组用逗号或 JSON，数字和布尔值用 JSON")
    p.add_argument("--page", type=int, help="起始页码（默认 1）")
    p.add_argument("--size", type=int, help="每页条数（默认 200，上限 200）")
    p.add_argument("--no-all-pages", action="store_true",
                   help="只取一页（默认自动翻页取全量）")
    p.add_argument("--max-pages", type=int, default=MAX_PAGES_DEFAULT,
                   help=f"最多翻页数，默认 {MAX_PAGES_DEFAULT}")
    p.add_argument("--time-cycle", dest="time_cycle", help="ALL/YEAR/MONTH/DAY/HOUR")
    p.add_argument("--metric", help="指标编码（cityMetric / channelCompare 必填）")
    p.add_argument("--channel", help="DINE/TAKEOUT/SELF_PICK")
    p.add_argument("--channel-list", dest="channel_list", help="多渠道，逗号分隔")
    p.add_argument("--time-slot", dest="time_slot", help="AllDay/Lunch/Afternoon/Dinner/Night/Other")
    p.add_argument("--dimension", help="评价分析维度（枚举名或中文）")
    p.add_argument("--granularity", help="DAY/HOUR")
    p.add_argument("--summary-type", dest="summary_type", help="CATEGORY/SINGLE/COMBO")
    p.add_argument("--baseline-weeks", type=int, help="基准周数 1~12，默认 4")
    p.add_argument("--note-type", dest="note_type", help="row_note/source_note")
    p.add_argument("--brand-names", dest="brand_names", help="peer-metric 品牌名，逗号分隔")
    p.add_argument("--period-labels", dest="period_labels", help="peer-metric 期间，如 25年,26H1")
    p.add_argument("--metric-keyword", dest="metric_keyword", help="peer-metric 指标关键字")
    p.add_argument("--compare-type", dest="compare_type", help="peer-metric: value/vs_taier")
    p.add_argument("--section-name", dest="section_name", help="peer-metric: 营收端/成本端/经营利润端")


def parse_parameters(items):
    body = {}
    for item in items or ():
        key, separator, value = item.partition("=")
        if not separator or not key.strip():
            raise SystemExit("[参数错误] --param 需要 K=V。")
        key = key.strip()
        if key in body:
            raise SystemExit(f"[参数错误] 字段 {key} 重复。")
        if key.endswith("List") and not value.lstrip().startswith("["):
            body[key] = split_list(value)
        else:
            try:
                body[key] = json.loads(value)
            except ValueError:
                body[key] = value
    return body


def write_result(command, result, args, extra=None):
    data = result.data
    payload = {
        "ok": result.error is None, "api": command, "complete": result.complete,
        "pagesFetched": result.pages, **(extra or {}),
    }
    if result.error:
        payload["error"] = {"code": result.error.code, "message": str(result.error)}
    if result.total_page is not None:
        payload.update(totalNum=result.total_num, totalPage=result.total_page)
    if isinstance(data, list):
        data = project_rows(data, args.fields)
        payload["rowCount"] = len(data)
    payload["data"] = data
    if args.out:
        target = Path(args.out).resolve()
        target.parent.mkdir(parents=True, exist_ok=True)
        if target.suffix.lower() == ".csv":
            if not isinstance(data, list):
                raise SystemExit("[输出错误] 单对象响应使用 JSON 导出。")
            write_csv(target, data)
        else:
            write_json(target, payload)
        payload["savedTo"] = str(target)
    if isinstance(data, list) and args.limit > 0 and len(data) > args.limit:
        payload["data"] = data[:args.limit]
        payload["previewOmitted"] = len(data) - args.limit
    if args.format == "table" and isinstance(data, list):
        print(render_table(payload["data"]))
        print(f"记录 {len(data)} 条，覆盖完整={result.complete}，已读取 {result.pages} 页", file=sys.stderr)
        if result.error:
            print(f"{result.error.code}: {result.error}", file=sys.stderr)
    else:
        print(json.dumps(payload, ensure_ascii=False, indent=2 if args.pretty else None))
    return int(result.error is not None)


def prepare_body(command, spec, args):
    if args.environment == "prod" and args.brand_no is not None:
        raise SystemExit("[参数错误] 正式环境按本人首个可用品牌查店，请省略 --brand-no。")
    if command == SHOP_TREE:
        return shop_tree_body(args.brand_no), None
    raw = parse_parameters(args.param)
    body = build_body(spec, args, raw)
    if spec["paged"]:
        page, size = body.setdefault("page", 1), body.setdefault("size", PAGE_SIZE_DEFAULT)
        if type(page) is not int or page < 1 or type(size) is not int or not 1 <= size <= PAGE_SIZE_DEFAULT:
            raise SystemExit("[参数错误] page 必须是正整数，size 范围为 1–200。")
    codes = split_list(args.shop_codes)
    if "shopCodeList" in body:
        supplied = body["shopCodeList"]
        if not isinstance(supplied, list) or any(not isinstance(code, str) for code in supplied):
            raise SystemExit("[参数错误] shopCodeList 必须是字符串数组。")
        if codes and codes != supplied:
            raise SystemExit("[参数错误] --shop-codes 与 shopCodeList 不一致。")
        codes = supplied
    if codes and spec["shops"] == "never":
        raise SystemExit("[参数错误] 本接口不按门店筛选。")
    if any(not VALID_SHOP_CODE.fullmatch(code) for code in codes):
        raise SystemExit("[参数错误] 门店组织编码格式不正确。")
    names = split_list(args.shop_name)
    if spec["shops"] == "never" and names:
        raise SystemExit("[参数错误] 本接口不按门店筛选。")
    if args.brand_no is not None and not names:
        raise SystemExit("[参数错误] 经营查询的 --brand-no 用于解析门店全名，请同时提供 --shop-name。")
    lookup = (names, codes, shop_tree_body(args.brand_no)) if names else None
    if codes:
        body["shopCodeList"] = codes
    validate(command, spec, body, codes, pending_lookup=lookup is not None)
    return body, lookup


def build_parser():
    parser = Parser(description="经 dt-cli 读取餐饮军师经营与评价数据")
    commands = parser.add_subparsers(dest="cmd", required=True)
    endpoints = commands.add_parser("endpoints", help="离线查看接口命令")
    endpoints.add_argument("--format", choices=["json", "table"], default="json")
    for name, spec in ENDPOINTS.items():
        aliases = [key for key, target in ALIASES.items() if target == name]
        sub = commands.add_parser(name, aliases=aliases, help=spec["desc"])
        add_shop_selection(sub)
        add_output(sub)
        if name != SHOP_TREE:
            add_common(sub)
        sub.add_argument("--cli", default="dt-cli", help="dt-cli 原生程序的绝对路径，或 PATH 中的 dt-cli")
        sub.add_argument("--profile", help="所选环境中已确认的本人平台记录")
        sub.add_argument("--environment", choices=["prod", "stg"], default="prod")
    return parser


def main():
    args = build_parser().parse_args()
    command = ALIASES.get(args.cmd, args.cmd)
    if command == "endpoints":
        rows = [{"command": name, "alias": alias(name), "path": spec["path"],
                 "paged": spec["paged"], "desc": spec["desc"]} for name, spec in ENDPOINTS.items()]
        print(render_table(rows) if args.format == "table" else json.dumps(rows, ensure_ascii=False))
        return 0
    if getattr(args, "max_pages", 1) < 1 or args.timeout < 1 or args.limit < 0:
        raise SystemExit("[参数错误] max-pages、timeout 必须为正数，limit 不得为负数。")
    spec = ENDPOINTS[command]
    body, lookup = prepare_body(command, spec, args)
    if args.dry_run:
        print(json.dumps({"ok": True, "dryRun": True, "system": SYSTEM, "alias": alias(command),
                          "environment": args.environment, "body": body,
                          "shopLookup": lookup[2] if lookup is not None else None},
                         ensure_ascii=False, indent=2))
        return 0
    if not args.profile:
        raise SystemExit("[缺少登录记录] 用 dt-cli profiles 核对本人及环境，再传 --profile。")
    client = PlatformClient(args.cli, args.profile, args.environment, args.timeout)
    try:
        if command == SHOP_TREE:
            names, codes = split_list(args.shop_name), split_list(args.shop_codes)
            rows = get_shops(client, body)
            rows = [asdict(shop) for shop in rows if (not names or shop.shopName in names)
                    and (not codes or shop.shopCode in codes)]
            return write_result(command, ReadResult(data=rows, pages=1, complete=True), args)
        client.resolve(command, spec)
        if lookup is not None:
            names, explicit_codes, lookup_body = lookup
            codes = resolve_codes(get_shops(client, lookup_body), names, explicit_codes)
            body["shopCodeList"] = codes
            validate(command, spec, body, codes)
        result = client.read(command, spec, body, 1 if args.no_all_pages else args.max_pages)
        return write_result(command, result, args)
    except PlatformError as exc:
        print(json.dumps({"ok": False, "api": command,
                          "error": {"code": exc.code, "message": str(exc)}}, ensure_ascii=False))
        return 1


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
    sys.exit(main())
