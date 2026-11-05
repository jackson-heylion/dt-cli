//! JSON只在分页传输边界提取声明的元数据；原业务响应完整保存在私有结果中。
use super::super::{contract::*, executor, input_model::WireInvocation};
use super::model::Progress;
use crate::output::{Failure, Result};
use serde_json::Value;

pub(super) struct Observation {
    /// 本页列表实际记录数。
    pub records: u64,
    /// 下一页页号。
    pub next_page: u64,
    /// 下一页游标。
    pub next_cursor: Option<String>,
    /// 已由规则明确识别结束。
    pub ended: bool,
}
pub(super) fn request(
    c: &ExecutionContract,
    original: &WireInvocation,
    progress: &Progress,
) -> Result<WireInvocation> {
    let mut request = original.clone();
    let paging = &c.pagination;
    if paging.kind == PaginationKind::None {
        return Ok(request);
    }
    let name = paging
        .page_parameter
        .as_deref()
        .filter(|v| !v.is_empty())
        .ok_or_else(invalid)?;
    let page = match paging.kind {
        PaginationKind::Page => Some(progress.next_page.to_string()),
        PaginationKind::Cursor => progress.next_cursor.clone(),
        PaginationKind::None => None,
    };
    // 页码通常按 (page - 1) * size 计算偏移，剩余额度不能改变固定页大小。
    let size = match paging.kind {
        PaginationKind::Page => u64::from(paging.page_size),
        _ => (c.limits.max_records - progress.records).min(paging.page_size.into()),
    };
    match paging.placement {
        PaginationPlacement::Query => {
            request.query = replace(&request.query, name, page.as_deref())?;
            if let Some(key) = &paging.size_parameter {
                request.query = replace(&request.query, key, Some(&size.to_string()))?;
            }
        }
        PaginationPlacement::Form => {
            request.body = replace(&request.body, name, page.as_deref())?;
            if let Some(key) = &paging.size_parameter {
                request.body = replace(&request.body, key, Some(&size.to_string()))?;
            }
        }
        PaginationPlacement::JsonBody => {
            let mut document: Value = serde_json::from_str(&request.body).map_err(|_| invalid())?;
            let value = match paging.kind {
                PaginationKind::Page => Some(Value::from(progress.next_page)),
                _ => page.map(Value::String),
            };
            set(&mut document, name, value)?;
            if let Some(key) = &paging.size_parameter {
                set(&mut document, key, Some(Value::from(size)))?;
            }
            request.body = serde_json::to_string(&document).map_err(|_| invalid())?;
        }
    }
    Ok(request)
}
fn replace(raw: &str, name: &str, value: Option<&str>) -> Result<String> {
    if name.contains(['&', '=']) {
        return Err(invalid());
    }
    let pairs: Vec<_> = url::form_urlencoded::parse(raw.as_bytes())
        .filter(|(key, _)| key != name)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        serializer.append_pair(&key, &value);
    }
    if let Some(value) = value {
        serializer.append_pair(name, value);
    }
    Ok(serializer.finish())
}
fn set(document: &mut Value, pointer: &str, value: Option<Value>) -> Result<()> {
    if !pointer.starts_with('/') || pointer.len() > 512 {
        return Err(invalid());
    }
    let split = pointer.rfind('/').ok_or_else(invalid)?;
    let name = pointer[split + 1..].replace("~1", "/").replace("~0", "~");
    let parent = if split == 0 {
        document
    } else {
        document
            .pointer_mut(&pointer[..split])
            .ok_or_else(invalid)?
    };
    let object = parent.as_object_mut().ok_or_else(invalid)?;
    match value {
        Some(value) => {
            object.insert(name, value);
        }
        None => {
            object.remove(&name);
        }
    }
    Ok(())
}
pub(super) fn observe(
    c: &ExecutionContract,
    body: &str,
    progress: &Progress,
) -> Result<Observation> {
    let paging = &c.pagination;
    if paging.kind == PaginationKind::None {
        let count = if body.is_empty() {
            0
        } else {
            let document: Value = serde_json::from_str(body).map_err(|_| invalid())?;
            if let Some(pointer) = &paging.items_pointer {
                document
                    .pointer(pointer)
                    .and_then(Value::as_array)
                    .ok_or_else(invalid)?
                    .len() as u64
            } else {
                document.as_array().map_or(1, |items| items.len() as u64)
            }
        };
        return Ok(Observation {
            records: count,
            next_page: progress.next_page,
            next_cursor: None,
            ended: true,
        });
    }
    let document: Value = serde_json::from_str(body).map_err(|_| invalid())?;
    let pointer = paging.items_pointer.as_deref().ok_or_else(invalid)?;
    let records = document
        .pointer(pointer)
        .and_then(Value::as_array)
        .ok_or_else(invalid)?
        .len() as u64;
    if paging.kind == PaginationKind::Cursor {
        let pointer = paging.next_pointer.as_deref().ok_or_else(invalid)?;
        let next = document.pointer(pointer).ok_or_else(invalid)?;
        let cursor = if next.is_null() {
            None
        } else if let Some(v) = next.as_str() {
            if v.is_empty() {
                None
            } else {
                Some(v.to_owned())
            }
        } else if next.is_number() {
            Some(next.to_string())
        } else {
            return Err(invalid());
        };
        if let Some(value) = &cursor {
            if progress.next_cursor.as_ref() == Some(value)
                || value.len() > 4096
                || progress
                    .cursor_digests
                    .contains(&executor::digest(value.as_bytes()))
            {
                return Err(Failure::new(
                    "PAGINATION_LOOP",
                    6,
                    "上游重复游标或游标超限，任务未宣称完整。",
                ));
            }
        }
        return Ok(Observation {
            records,
            next_page: progress.next_page,
            next_cursor: cursor.clone(),
            ended: cursor.is_none(),
        });
    }
    let mut ended = false;
    let mut next_page = progress.next_page.checked_add(1).ok_or_else(invalid)?;
    if let Some(pointer) = &paging.next_pointer {
        let next = document.pointer(pointer).ok_or_else(invalid)?;
        if next.is_null() {
            ended = true;
        } else {
            next_page = next.as_u64().ok_or_else(invalid)?;
            if next_page <= progress.next_page {
                return Err(invalid());
            }
        }
    } else if let Some(pointer) = &paging.total_pointer {
        let total = document
            .pointer(pointer)
            .and_then(Value::as_u64)
            .ok_or_else(invalid)?;
        ended = progress.records + records >= total;
        if records == 0 && !ended {
            return Err(invalid());
        }
    } else {
        ended = records < u64::from(paging.page_size);
    }
    Ok(Observation {
        records,
        next_page,
        next_cursor: None,
        ended,
    })
}
fn invalid() -> Failure {
    Failure::new(
        "PAGINATION_CONTRACT_MISMATCH",
        6,
        "分页响应、位置或结束条件不符合原固定规则；已完成页保留，未切换执行版本。",
    )
}

pub(super) fn initial(c: &ExecutionContract, request: &WireInvocation) -> Result<Progress> {
    let p = &c.pagination;
    let mut page = p.first_page;
    let mut cursor = None;
    if p.kind != PaginationKind::None {
        let key = p.page_parameter.as_deref().ok_or_else(invalid)?;
        let value = match p.placement {
            PaginationPlacement::Query | PaginationPlacement::Form => {
                let raw = if p.placement == PaginationPlacement::Query {
                    &request.query
                } else {
                    &request.body
                };
                let values: Vec<_> = url::form_urlencoded::parse(raw.as_bytes())
                    .filter(|(name, _)| name == key)
                    .map(|(_, value)| value.into_owned())
                    .collect();
                if values.len() > 1 {
                    return Err(invalid());
                }
                values.into_iter().next()
            }
            PaginationPlacement::JsonBody => {
                let doc: Value = serde_json::from_str(&request.body).map_err(|_| invalid())?;
                match doc.pointer(key) {
                    None | Some(Value::Null) => None,
                    Some(Value::String(v)) => Some(v.clone()),
                    Some(Value::Number(v)) => Some(v.to_string()),
                    _ => return Err(invalid()),
                }
            }
        };
        if p.kind == PaginationKind::Page {
            if let Some(value) = value {
                page = value.parse().map_err(|_| invalid())?;
                if page < p.first_page {
                    return Err(invalid());
                }
            }
        } else {
            cursor = value;
        }
    }
    Ok(Progress {
        pages: 0,
        records: 0,
        bytes: 0,
        active_millis: 0,
        pending_budget_millis: None,
        next_page: page,
        next_cursor: cursor,
        cursor_digests: vec![],
    })
}
