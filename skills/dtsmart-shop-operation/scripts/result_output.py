"""将经过校验的业务结果写成 JSON、CSV 或终端表格。"""

import csv
import json
import os
import unicodedata


def project_rows(rows, fields):
    if not fields:
        return rows
    keep = [f.strip() for f in fields.split(",") if f.strip()]
    out = []
    for row in rows:
        if isinstance(row, dict):
            out.append({k: row[k] for k in keep if k in row})
        else:
            out.append(row)
    return out


def flatten(value):
    if isinstance(value, (dict, list)):
        return json.dumps(value, ensure_ascii=False)
    return value


def write_csv(path, rows):
    rows = [r for r in rows if isinstance(r, dict)]
    if not rows:
        raise SystemExit("[无数据] 结果为空，未写出 CSV")
    fieldnames = []
    for row in rows:
        for key in row:
            if key not in fieldnames:
                fieldnames.append(key)
    with open(path, "w", encoding="utf-8-sig", newline="") as fh:
        writer = csv.DictWriter(fh, fieldnames=fieldnames, extrasaction="ignore")
        writer.writeheader()
        for row in rows:
            writer.writerow({k: flatten(row.get(k)) for k in fieldnames})


def write_json(path, payload):
    os.makedirs(os.path.dirname(os.path.abspath(path)) or ".", exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(payload, fh, ensure_ascii=False, indent=2)


def disp_len(text):
    """按终端显示宽度计算（CJK 等宽字符按 2 计）。"""
    return sum(2 if unicodedata.east_asian_width(ch) in ("W", "F", "A") else 1 for ch in text)


def pad(text, width):
    return text + " " * max(0, width - disp_len(text))


def render_table(rows, max_width=42):
    rows = [r for r in rows if isinstance(r, dict)]
    if not rows:
        return "(空)"
    cols = []
    for row in rows:
        for key in row:
            if key not in cols:
                cols.append(key)
    cells = [[("" if r.get(c) is None else str(flatten(r.get(c)))) for c in cols] for r in rows]
    widths = []
    for i, col in enumerate(cols):
        w = max([disp_len(col)] + [disp_len(row[i]) for row in cells])
        widths.append(min(w, max_width))

    def cut(text, width):
        if disp_len(text) <= width:
            return text
        out = ""
        for ch in text:
            if disp_len(out + ch) > width - 1:
                break
            out += ch
        return out + "…"

    def line(values):
        return "  ".join(pad(cut(v, widths[i]), widths[i]) for i, v in enumerate(values)).rstrip()

    out = [line(cols), "  ".join("-" * widths[i] for i in range(len(cols)))]
    for row in cells:
        out.append(line(row))
    return "\n".join(out)
