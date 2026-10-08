use super::*;

pub(super) fn read_request(stream: &mut std::net::TcpStream) -> Option<(String, Value)> {
    use std::io::Read;
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .ok()?;
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        data.extend_from_slice(&chunk[..n]);
        if let Some(position) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break position + 4;
        }
        if data.len() > 1 << 20 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&data[..head_end]).to_string();
    let path = head.lines().next()?.split(' ').nth(1)?.to_string();
    let length = head
        .lines()
        .find_map(|line| {
            line.to_lowercase()
                .strip_prefix("content-length:")
                .map(|n| n.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    while data.len() < head_end + length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..n]);
    }
    let body = if length > 0 && data.len() >= head_end + length {
        serde_json::from_slice(&data[head_end..head_end + length]).unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    Some((path, body))
}

pub(super) fn serve<F>(handler: F) -> Server
where
    F: Fn(usize, &str, &Value) -> Reply + Send + Sync + 'static,
{
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let state: Arc<Mutex<SharedState>> = Arc::new(Mutex::new(SharedState::default()));
    let shared = state.clone();
    let handler = Arc::new(handler);
    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(mut stream) = incoming else { break };
            let state = shared.clone();
            let handler = handler.clone();
            std::thread::spawn(move || {
                use std::io::Write;
                let Some((path, body)) = read_request(&mut stream) else {
                    return;
                };
                let index = {
                    let mut state = state.lock().unwrap();
                    state.entries.push((path.clone(), body.clone()));
                    state.entries.len() - 1
                };
                let reply = handler(index, &path, &body);
                if reply.delay_ms > 0 {
                    std::thread::sleep(Duration::from_millis(reply.delay_ms));
                }
                if reply.abort {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    return;
                }
                let mut head = format!(
                    "HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
                    reply.status,
                    reply.body.len()
                );
                for (name, value) in &reply.headers {
                    head.push_str(&format!("{name}: {value}\r\n"));
                }
                head.push_str("\r\n");
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&reply.body);
            });
        }
    });
    Server { origin, state }
}
