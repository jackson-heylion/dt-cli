use super::*;

pub(super) struct Iam {
    pub(super) origin: String,
    pub(super) fake: Arc<Mutex<Fake>>,
}
impl Iam {
    pub(super) fn start(system: &str, environment: &str, operations: Vec<Value>) -> Self {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let fake = Arc::new(Mutex::new(Fake::new(system, environment, operations)));
        fake.lock().unwrap().origin = origin.clone();
        let shared = fake.clone();
        std::thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(stream) = incoming else { break };
                let fake = shared.clone();
                std::thread::spawn(move || serve(stream, fake));
            }
        });
        Self { origin, fake }
    }
    pub(super) fn with<T>(&self, change: impl FnOnce(&mut Fake) -> T) -> T {
        change(&mut self.fake.lock().unwrap())
    }
    pub(super) fn seen(&self) -> Vec<Seen> {
        self.with(|f| f.seen.clone())
    }
    pub(super) fn count(&self) -> usize {
        self.with(|f| f.seen.len())
    }
    pub(super) fn since(&self, start: usize) -> Vec<Seen> {
        self.seen()[start..].to_vec()
    }
    pub(super) fn paths_since(&self, start: usize) -> Vec<String> {
        self.since(start)
            .into_iter()
            .map(|s| format!("{} {}", s.method, s.path))
            .collect()
    }
}
pub(super) fn serve(mut stream: std::net::TcpStream, fake: Arc<Mutex<Fake>>) {
    use std::io::{Read, Write};
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let Ok(n) = stream.read(&mut chunk) else {
            return;
        };
        if n == 0 {
            return;
        }
        data.extend_from_slice(&chunk[..n]);
        if let Some(position) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8_lossy(&data[..head_end]).to_string();
    let mut lines = head.lines();
    let first: Vec<_> = lines.next().unwrap_or_default().split(' ').collect();
    let mut length = 0;
    let mut authorization = None;
    let mut idempotency = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            match name.trim().to_lowercase().as_str() {
                "content-length" => length = value.trim().parse().unwrap_or(0),
                "authorization" => authorization = Some(value.trim().to_owned()),
                "idempotency-key" => idempotency = Some(value.trim().to_owned()),
                _ => {}
            }
        }
    }
    while data.len() < head_end + length {
        let Ok(n) = stream.read(&mut chunk) else {
            return;
        };
        if n == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..n]);
    }
    let request = Seen {
        method: first.first().copied().unwrap_or_default().to_owned(),
        path: first.get(1).copied().unwrap_or_default().to_owned(),
        authorization,
        idempotency,
        body: String::from_utf8_lossy(&data[head_end..(head_end + length).min(data.len())])
            .to_string(),
    };
    let (status, headers, payload) = {
        let mut fake = fake.lock().unwrap();
        fake.seen.push(request.clone());
        fake.handle(&request)
    };
    // Status 0 closes without an answer; 1 stalls first, as a slow or lost write response.
    if status <= 1 {
        if status == 1 {
            std::thread::sleep(Duration::from_secs(5));
        }
        return;
    }
    let mut head = format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
        payload.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&payload);
}
