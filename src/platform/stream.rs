use super::{contract::Sse, errors, invocation, model::RemoteError};
use crate::output::{Failure, PartialData, PartialMeta, Result, envelope_result};
use serde::Serialize;
use std::time::Duration;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;

const IDLE: Duration = Duration::from_secs(45);

#[derive(Serialize)]
struct Event {
    /// SSE 事件名；缺省为 message。
    event: String,
    /// 本事件显式携带的 ID。
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    /// 多行 data 依照 SSE 规则连接后的正文。
    body: String,
}

#[derive(Default)]
struct Decoder {
    line: Vec<u8>,
    frame_bytes: usize,
    first_line: bool,
    skip_lf: bool,
    comment: bool,
    has_data: bool,
    event: String,
    id: Option<String>,
    data: String,
}

enum Frame {
    Event(Event),
    Heartbeat,
}

impl Decoder {
    fn byte(&mut self, byte: u8, limit: usize) -> Result<Option<Frame>> {
        if std::mem::take(&mut self.skip_lf) && byte == b'\n' {
            if self.frame_bytes != 0 {
                self.frame_bytes += 1;
                if self.frame_bytes > limit {
                    return Err(too_large());
                }
            }
            return Ok(None);
        }
        self.frame_bytes += 1;
        if self.frame_bytes > limit {
            return Err(too_large());
        }
        if byte != b'\r' && byte != b'\n' {
            self.line.push(byte);
            return Ok(None);
        }
        self.skip_lf = byte == b'\r';
        let line = std::str::from_utf8(&self.line).map_err(|_| invalid_event())?;
        let line = if self.first_line {
            self.first_line = false;
            line.strip_prefix('\u{feff}').unwrap_or(line)
        } else {
            line
        };

        let mut frame = None;
        if line.is_empty() {
            if self.has_data {
                self.data.pop();
                frame = Some(Frame::Event(Event {
                    event: if self.event.is_empty() {
                        "message".into()
                    } else {
                        std::mem::take(&mut self.event)
                    },
                    id: self.id.take(),
                    body: std::mem::take(&mut self.data),
                }));
            } else if self.comment {
                frame = Some(Frame::Heartbeat);
            }
            self.frame_bytes = 0;
            self.has_data = false;
            self.comment = false;
            self.event.clear();
            self.id = None;
        } else if line.starts_with(':') {
            self.comment = true;
        } else {
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => self.event = value.to_owned(),
                "id" if !value.contains('\0') => self.id = Some(value.to_owned()),
                "data" => {
                    self.has_data = true;
                    self.data.push_str(value);
                    self.data.push('\n');
                }
                _ => {}
            }
        }
        self.line.clear();
        Ok(frame)
    }
}

fn invalid_event() -> Failure {
    Failure::new("STREAM_CONTRACT_MISMATCH", 6, "SSE 事件不符合固定契约。")
}

fn too_large() -> Failure {
    Failure::new("RESPONSE_LIMIT", 6, "SSE 事件超过固定执行版本的大小上限。")
}

fn interrupted() -> Failure {
    Failure::new("STREAM_INTERRUPTED", 6, "订阅提前结束，请核对原执行状态。")
}

fn cancelled() -> Failure {
    Failure::new("STREAM_CANCELLED", 130, "已停止监听，请核对原执行状态。")
}

fn output_closed() -> Failure {
    Failure::new("STREAM_OUTPUT_CLOSED", 6, "无法继续写入流式输出。")
}

pub(super) fn validator(sse: &Sse) -> Result<jsonschema::Validator> {
    if sse.max_event_bytes == 0
        || sse.max_event_bytes > crate::constants::execution::RESPONSE_BYTES
        || !(1..=1800).contains(&sse.max_connection_seconds)
        || sse.terminal_events.is_empty()
    {
        return Err(invalid_event());
    }
    invocation::schema_validator(&sse.event_schema)
}

pub(super) async fn consume(
    mut response: reqwest::Response,
    sse: &Sse,
    validator: &jsonschema::Validator,
    profile: &str,
    stdout: &mut (dyn AsyncWrite + Unpin),
) -> Result<()> {
    use reqwest::header::CONTENT_TYPE;
    if response.status().as_u16() != 200 {
        let headers = response.headers();
        let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
        let status = response.status().as_u16();
        let trace = errors::trace(header("X-Trace-Id"));
        let mut failure = errors::gateway(
            status,
            header("X-Platform-Error-Code"),
            None,
            trace.as_deref(),
        );
        if matches!(status, 410 | 503)
            && header("X-Platform-Error-Code").is_none()
            && let Ok(Ok(Some(remote))) =
                tokio::time::timeout(Duration::from_secs(5), remote_error(&mut response)).await
        {
            failure = errors::gateway(status, remote.code.as_deref(), None, trace.as_deref());
        }
        return Err(failure);
    }
    if !response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"))
    {
        return Err(invalid_event());
    }
    let mut last_event_id = None;
    let mut decoder = Decoder {
        first_line: true,
        ..Decoder::default()
    };
    let total = Instant::now() + Duration::from_secs(sse.max_connection_seconds);
    let mut idle = Instant::now() + IDLE;
    let result = async {
        loop {
            let chunk = tokio::select! {
                _ = tokio::signal::ctrl_c() => return Err(cancelled()),
                _ = tokio::time::sleep_until(total) => return Err(Failure::new("STREAM_DURATION_LIMIT", 6, "已达到单条连接时限。")),
                _ = tokio::time::sleep_until(idle) => return Err(Failure::new("STREAM_IDLE_TIMEOUT", 6, "长时间未收到有效事件或心跳。")),
                chunk = response.chunk() => chunk.map_err(|_| interrupted())?.ok_or_else(interrupted)?,
            };
            for &byte in &chunk {
                if let Some(frame) = decoder.byte(byte, sse.max_event_bytes)? {
                    idle = Instant::now() + IDLE;
                    let Frame::Event(event) = frame else { continue };
                    let data = serde_json::from_str(&event.body).map_err(|_| invalid_event())?;
                    if !validator.is_valid(&data) {
                        return Err(invalid_event());
                    }
                    let terminal = sse.terminal_events.contains(&event.event);
                    let id = event.id.clone().filter(|id| !id.is_empty());
                    let (record, _) = envelope_result(
                        "platform.call", Some(profile),
                        Ok(serde_json::to_value(event).map_err(|_| invalid_event())?),
                    );
                    let mut line = serde_json::to_vec(&record).map_err(|_| invalid_event())?;
                    line.push(b'\n');
                    // 写入可能停在下游管道。超时或中断时立即释放响应，避免继续读上游。
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => return Err(output_closed()),
                        _ = tokio::time::sleep_until(total) => return Err(output_closed()),
                        written = async { stdout.write_all(&line).await?; stdout.flush().await } =>
                            written.map_err(|_| output_closed())?,
                    }
                    if id.is_some() {
                        last_event_id = id;
                    }
                    if terminal {
                        return Ok(());
                    }
                }
            }
        }
    }.await;
    result.map_err(|failure| {
        failure.with_partial(
            PartialData::Empty,
            PartialMeta {
                complete: Some(false),
                last_event_id,
                ..Default::default()
            },
        )
    })
}

async fn remote_error(response: &mut reqwest::Response) -> Result<Option<RemoteError>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| interrupted())? {
        if body.len() + chunk.len() > 65536 {
            return Ok(None);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&body).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frames_across_splits_preserving_data_and_explicit_ids() {
        let mut decoder = Decoder {
            first_line: true,
            ..Decoder::default()
        };
        let input = "\u{feff}: heartbeat\r\n\r\nid: run:3\r\nevent: delta\r\ndata: {\r\ndata:  \"x\": 1}\r\n\r\ndata: {}\n\n";
        let events: Vec<_> = input
            .bytes()
            .filter_map(|byte| decoder.byte(byte, 256).unwrap())
            .collect();
        assert!(
            matches!(events.as_slice(), [Frame::Heartbeat, Frame::Event(first), Frame::Event(second)]
            if first.id.as_deref() == Some("run:3") && first.event == "delta"
            && first.body == "{\n \"x\": 1}" && second.id.is_none() && second.event == "message")
        );
    }

    #[test]
    fn rejects_unterminated_oversize_frames() {
        let mut decoder = Decoder::default();
        for byte in b"data: " {
            assert!(decoder.byte(*byte, 6).unwrap().is_none());
        }
        assert_eq!(decoder.byte(b'a', 6).err().unwrap().code, "RESPONSE_LIMIT");
    }
}
