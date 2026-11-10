//! 模板预览与显式摘要绑定的文件替换，默认不覆盖。
use super::Detail;
use crate::output::{Failure, Result, invalid};
use crate::output::{PartialData, PartialMeta};
use sha2::{Digest, Sha256};
use std::{io::Write, path::Path};
pub(super) fn export(
    detail: &Detail,
    o: &crate::interface::request::Template,
) -> Result<serde_json::Value> {
    // 模板必须可解码为既有具名调用输入，禁止服务端扩展被静默写入。
    let _: super::super::input_model::InvocationInput =
        serde_json::from_str(&detail.example.input_json).map_err(|_| crate::http::protocol())?;
    let Some(output) = o.output.as_ref() else {
        return super::super::serialize(detail);
    };
    let path = Path::new(output);
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 2_097_152 {
            return Err(invalid());
        }
        let bytes = std::fs::read(path).map_err(|_| crate::private_store::unavailable())?;
        let digest = format!("{:x}", Sha256::digest(bytes));
        if o.overwrite_digest.as_ref() != Some(&digest) {
            return Err(Failure::new(
                "TEMPLATE_FILE_EXISTS",
                2,
                "已有文件未覆盖；请先预览模板，明确原文件摘要后再替换。",
            )
            .with_partial(
                PartialData::Template(detail.clone()),
                PartialMeta {
                    existing_digest: Some(digest),
                    next_action: Some("REVIEW_BEFORE_OVERWRITE"),
                    ..Default::default()
                },
            ));
        }
    } else if o.overwrite_digest.is_some() {
        return Err(invalid());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| crate::private_store::unavailable())?;
    temporary
        .write_all(detail.example.input_json.as_bytes())
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| crate::private_store::unavailable())?;
    if o.overwrite_digest.is_some() {
        temporary
            .persist(path)
            .map_err(|_| crate::private_store::unavailable())?;
    } else {
        temporary
            .persist_noclobber(path)
            .map_err(|_| Failure::new("TEMPLATE_FILE_EXISTS", 2, "目标文件已存在，未覆盖。"))?;
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Export<'a> {
        /// 已保存的公开模板路径。
        output: &'a str,
        /// 固定契约及需要填写的值。
        detail: &'a Detail,
    }
    super::super::serialize(Export { output, detail })
}
