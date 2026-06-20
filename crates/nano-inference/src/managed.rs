use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use url::Url;

use crate::{
    InferError, InferRequest, InferResponse, MockPredictor, ModelMeta, Predictor, RemotePredictor,
    WireApi,
};

#[derive(Debug, Clone)]
pub enum LaunchRecipe {
    Command {
        command: String,
        args: Vec<String>,
        port: u16,
        health_timeout: Duration,
    },
    BuiltInMock {
        port: u16,
        health_timeout: Duration,
    },
}

impl LaunchRecipe {
    pub fn builtin_mock_ephemeral() -> Self {
        Self::BuiltInMock {
            port: 0,
            health_timeout: Duration::from_secs(5),
        }
    }

    fn health_timeout(&self) -> Duration {
        match self {
            Self::Command { health_timeout, .. } | Self::BuiltInMock { health_timeout, .. } => {
                *health_timeout
            }
        }
    }
}

pub struct ManagedPredictor {
    remote: RemotePredictor,
    process: ManagedProcess,
}

impl ManagedPredictor {
    pub fn start(
        launch: LaunchRecipe,
        model: impl Into<String>,
        api: WireApi,
    ) -> Result<Self, InferError> {
        let model = model.into();
        let (process, port) = match &launch {
            LaunchRecipe::Command {
                command,
                args,
                port,
                ..
            } => {
                let child = Command::new(command)
                    .args(args)
                    .spawn()
                    .map_err(|err| InferError::Transport(err.to_string()))?;
                (ManagedProcess::Child(child), *port)
            }
            LaunchRecipe::BuiltInMock { port, .. } => {
                let server = BuiltInMockServer::start(&model, *port)?;
                let bound_port = server.port();
                (ManagedProcess::BuiltIn(server), bound_port)
            }
        };

        let endpoint = Url::parse(&format!("http://127.0.0.1:{port}/"))
            .map_err(|err| InferError::Transport(err.to_string()))?;
        poll_ready(&endpoint, launch.health_timeout())?;
        let remote = RemotePredictor::new(endpoint, model, api);
        Ok(Self { remote, process })
    }

    pub fn endpoint(&self) -> &Url {
        self.remote.endpoint()
    }

    pub fn managed_port(&self) -> u16 {
        self.endpoint().port().unwrap_or(80)
    }
}

impl Predictor for ManagedPredictor {
    fn predict(&self, req: &InferRequest) -> Result<InferResponse, InferError> {
        self.remote.predict(req)
    }

    fn metadata(&self) -> ModelMeta {
        self.remote.metadata()
    }
}

impl Drop for ManagedPredictor {
    fn drop(&mut self) {
        match &mut self.process {
            ManagedProcess::Child(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            ManagedProcess::BuiltIn(server) => server.stop(),
        }
    }
}

enum ManagedProcess {
    Child(Child),
    BuiltIn(BuiltInMockServer),
}

pub struct BuiltInMockServer {
    addr: SocketAddr,
    running: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl BuiltInMockServer {
    pub fn start(model: &str, port: u16) -> Result<Self, InferError> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .map_err(|err| InferError::Transport(err.to_string()))?;
        listener
            .set_nonblocking(true)
            .map_err(|err| InferError::Transport(err.to_string()))?;
        let addr = listener
            .local_addr()
            .map_err(|err| InferError::Transport(err.to_string()))?;
        let running = Arc::new(AtomicBool::new(true));
        let thread_running = Arc::clone(&running);
        let model = model.to_string();
        let join = thread::spawn(move || {
            let predictor = MockPredictor::new(&model);
            while thread_running.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => handle_client(stream, &model, &predictor),
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            addr,
            running,
            join: Some(join),
        })
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for BuiltInMockServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn poll_ready(endpoint: &Url, timeout: Duration) -> Result<(), InferError> {
    let deadline = Instant::now() + timeout;
    let url = endpoint
        .join("v2/health/ready")
        .map_err(|err| InferError::Transport(err.to_string()))?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_millis(250))
        .timeout_read(Duration::from_millis(250))
        .build();
    loop {
        match agent.get(url.as_str()).call() {
            Ok(response) if response.status() == 200 => return Ok(()),
            Ok(_) | Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(response) => {
                return Err(InferError::ServerStatus {
                    code: response.status(),
                    body: response.into_string().unwrap_or_default(),
                });
            }
            Err(_) => {
                return Err(InferError::Timeout(format!(
                    "health check {url} did not become ready within {timeout:?}"
                )));
            }
        }
    }
}

fn handle_client(mut stream: TcpStream, model: &str, predictor: &MockPredictor) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let response = match read_http_request(&mut stream) {
        Ok(request) => route_request(request, model, predictor),
        Err(err) => http_response(400, "text/plain", &err),
    };
    let _ = stream.write_all(response.as_bytes());
}

struct HttpRequest {
    method: String,
    path: String,
    body: String,
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut bytes = Vec::new();
    let mut header_end = None;
    let mut buf = [0u8; 1024];
    while header_end.is_none() {
        let read = stream.read(&mut buf).map_err(|err| err.to_string())?;
        if read == 0 {
            return Err("connection closed before headers".to_string());
        }
        bytes.extend_from_slice(&buf[..read]);
        header_end = find_header_end(&bytes);
        if bytes.len() > 64 * 1024 {
            return Err("request headers too large".to_string());
        }
    }

    let header_end = header_end.expect("checked header_end");
    let headers = String::from_utf8(bytes[..header_end].to_vec()).map_err(|err| err.to_string())?;
    let mut lines = headers.lines();
    let request_line = lines
        .next()
        .ok_or_else(|| "missing request line".to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let content_length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);

    let body_start = header_end + 4;
    while bytes.len() < body_start + content_length {
        let read = stream.read(&mut buf).map_err(|err| err.to_string())?;
        if read == 0 {
            return Err("connection closed before request body".to_string());
        }
        bytes.extend_from_slice(&buf[..read]);
    }
    let body = String::from_utf8(bytes[body_start..body_start + content_length].to_vec())
        .map_err(|err| err.to_string())?;
    Ok(HttpRequest { method, path, body })
}

fn route_request(request: HttpRequest, fallback_model: &str, predictor: &MockPredictor) -> String {
    if request.method == "GET" && request.path == "/v2/health/ready" {
        return http_response(200, "application/json", "{}");
    }

    if request.method == "POST"
        && request.path.starts_with("/v2/models/")
        && request.path.ends_with("/infer")
    {
        let model = request
            .path
            .strip_prefix("/v2/models/")
            .and_then(|tail| tail.strip_suffix("/infer"))
            .filter(|value| !value.is_empty())
            .unwrap_or(fallback_model)
            .to_string();
        let body = crate::remote::parse_kserve_request(&request.body, model);
        return match body
            .and_then(|req| predictor.predict(&req))
            .and_then(|response| crate::remote::serialize_kserve_response(&response))
        {
            Ok(json) => http_response(200, "application/json", &json),
            Err(err) => http_response(400, "text/plain", &err.to_string()),
        };
    }

    http_response(404, "text/plain", "not found")
}

fn http_response(status: u16, content_type: &str, body: &str) -> String {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}
