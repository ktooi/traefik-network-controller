//! Narrow Docker Engine transport. This module never consults DOCKER_HOST and
//! never opens TCP connections: both clients are pinned to one Unix socket.
//! Resource ownership and the desired set of network memberships live in the
//! policy/controller modules; the transport additionally restricts API shapes.

use std::path::Path;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use reqwest::{Client, Method, Response, StatusCode};
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tokio::time::{Instant, sleep, timeout};

const AUTHORITY: &str = "http://docker";
const MIN_API: (u16, u16) = (1, 48);
const MAX_API: (u16, u16) = (1, 56);
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_EVENT_LINE_BYTES: usize = 64 * 1024;
const MAX_REQUEST_PATH_BYTES: usize = 16 * 1024;
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const STABLE_EVENT_SESSION: Duration = Duration::from_secs(60);
const EVENT_FILTERS: &str = r#"{"type":["container","network"]}"#;

#[derive(Clone)]
pub struct Docker {
    client: Client,
    events_client: Client,
    request_timeout: Duration,
}

impl Docker {
    pub fn new(socket_path: &Path, request_timeout: Duration) -> Result<Self> {
        if !socket_path.is_absolute() {
            bail!("Docker socket path must be absolute");
        }
        if request_timeout.is_zero() {
            bail!("Docker request timeout must be positive");
        }

        // Do not retain a reqwest error as an anyhow source: formatting its
        // chain could otherwise disclose a URL containing labels or filters.
        let client = unix_client(socket_path, Some(request_timeout))?;
        let events_client = unix_client(socket_path, None)?;
        Ok(Self {
            client,
            events_client,
            request_timeout,
        })
    }

    /// Choose the highest API version in the intersection of the server's
    /// advertised range and this controller's supported 1.48..=1.56 range.
    pub async fn negotiate(&self) -> Result<String> {
        let response = self
            .client
            .get(format!("{AUTHORITY}/version"))
            .send()
            .await
            .map_err(|_| anyhow!("Docker API version request failed"))?;
        require_success(&response)?;
        let body = read_capped(response).await?;
        let version: Value = serde_json::from_slice(&body)
            .map_err(|_| anyhow!("Docker API version response is not valid JSON"))?;
        choose_api(&version)
    }

    pub async fn get(&self, api: &str, path: &str) -> Result<Value> {
        let response = self.send(Method::GET, api, path, None).await?;
        require_success(&response)?;
        decode_json(response, false).await
    }

    /// Only a real HTTP 404 means absence. Authorization, parsing, timeout,
    /// connection and daemon failures must never be mistaken for absence.
    pub async fn get_optional(&self, api: &str, path: &str) -> Result<Option<Value>> {
        let response = self.send(Method::GET, api, path, None).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        require_success(&response)?;
        decode_json(response, false).await.map(Some)
    }

    pub async fn post(&self, api: &str, path: &str, body: &Value) -> Result<Value> {
        validate_post_body(path, body)?;
        let response = self.send(Method::POST, api, path, Some(body)).await?;
        require_success(&response)?;
        decode_json(response, true).await
    }

    pub async fn delete(&self, api: &str, path: &str) -> Result<()> {
        let response = self.send(Method::DELETE, api, path, None).await?;
        require_success(&response)?;
        // Normal deletion responses are empty. Still bound any unexpected
        // successful response rather than buffering it without a limit.
        let _ = read_capped(response).await?;
        Ok(())
    }

    async fn send(
        &self,
        method: Method,
        api: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Response> {
        validate_api(api)?;
        validate_path(&method, path)?;
        let mut request = self
            .client
            .request(method, format!("{AUTHORITY}/v{api}{path}"));
        if let Some(body) = body {
            request = request.json(body);
        }
        request
            .send()
            .await
            .map_err(|_| anyhow!("Docker API request failed"))
    }

    /// Watch hints, not a durable event log. The controller must also perform
    /// startup and periodic reconciliation. A bounded channel coalesces bursts.
    /// The stream has an opening deadline but no total/idle body deadline.
    pub async fn watch_events(&self, tx: mpsc::Sender<()>, mut shutdown: watch::Receiver<bool>) {
        let mut backoff = INITIAL_BACKOFF;
        loop {
            if *shutdown.borrow() || tx.is_closed() {
                return;
            }
            let started = Instant::now();
            tokio::select! {
                biased;
                _ = wait_for_shutdown(&mut shutdown) => return,
                _ = tx.closed() => return,
                _ = self.event_session(&tx) => {}
            }
            // A disconnect can hide changes. Ask for a scan immediately,
            // including when opening/negotiating the stream failed.
            if !notify(&tx) {
                return;
            }
            if started.elapsed() >= STABLE_EVENT_SESSION {
                backoff = INITIAL_BACKOFF;
            }
            tokio::select! {
                biased;
                _ = wait_for_shutdown(&mut shutdown) => return,
                _ = tx.closed() => return,
                _ = sleep(backoff) => {}
            }
            backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
        }
    }

    async fn event_session(&self, tx: &mpsc::Sender<()>) -> Result<()> {
        // Re-negotiate after every disconnect; the daemon may have restarted
        // into a different version while this process remained running.
        let api = self.negotiate().await?;
        let request = self
            .events_client
            .get(format!("{AUTHORITY}/v{api}/events"))
            .query(&[("filters", EVENT_FILTERS)]);
        let mut response = timeout(self.request_timeout, request.send())
            .await
            .map_err(|_| anyhow!("Docker event stream opening timed out"))?
            .map_err(|_| anyhow!("Docker event stream opening failed"))?;
        require_success(&response)?;
        if !notify(tx) {
            return Ok(());
        }

        let mut lines = EventLines::default();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("Docker event stream read failed"))?
        {
            if !lines.push(&chunk, tx)? {
                return Ok(());
            }
        }
        // A last JSON record need not end with a newline. A truncated/malformed
        // one restarts the stream; the disconnect notification causes a scan.
        let _ = lines.finish(tx)?;
        Ok(())
    }
}

fn unix_client(socket_path: &Path, request_timeout: Option<Duration>) -> Result<Client> {
    let mut builder = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .http1_only()
        .unix_socket(socket_path);
    if let Some(request_timeout) = request_timeout {
        builder = builder.timeout(request_timeout);
    }
    builder
        .build()
        .map_err(|_| anyhow!("Docker Unix socket client could not be initialized"))
}

fn parse_api(version: &str) -> Option<(u16, u16)> {
    let (major, minor) = version.split_once('.')?;
    // Canonical decimal only: do not allow strings that become URL syntax.
    if major.is_empty()
        || minor.is_empty()
        || major.len() > 3
        || minor.len() > 3
        || (major.len() > 1 && major.starts_with('0'))
        || (minor.len() > 1 && minor.starts_with('0'))
        || !major.bytes().all(|b| b.is_ascii_digit())
        || !minor.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

fn choose_api(version: &Value) -> Result<String> {
    let server_max = version
        .get("ApiVersion")
        .and_then(Value::as_str)
        .and_then(parse_api)
        .ok_or_else(|| anyhow!("Docker server API range is missing or invalid"))?;
    let server_min = version
        .get("MinAPIVersion")
        .and_then(Value::as_str)
        .and_then(parse_api)
        .ok_or_else(|| anyhow!("Docker server API range is missing or invalid"))?;
    if server_min > server_max {
        bail!("Docker server API range is invalid");
    }
    let selected = server_max.min(MAX_API);
    if selected < server_min.max(MIN_API) {
        bail!("Docker server has no supported API version (controller supports 1.48-1.56)");
    }
    Ok(format!("{}.{}", selected.0, selected.1))
}

fn validate_api(api: &str) -> Result<()> {
    if !parse_api(api).is_some_and(|version| (MIN_API..=MAX_API).contains(&version)) {
        bail!("Docker API version is outside the supported range");
    }
    Ok(())
}

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

fn full_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn single_filter_query(query: Option<&str>) -> bool {
    match query {
        None => true,
        Some(query) => {
            let Some(encoded) = query.strip_prefix("filters=") else {
                return false;
            };
            if encoded.is_empty() {
                return false;
            }
            // The caller supplies a percent-encoded JSON filters argument.
            // Restrict its raw bytes to unreserved/form-encoded characters so
            // a second parameter, fragment or control byte cannot sneak in.
            let mut bytes = encoded.bytes();
            while let Some(byte) = bytes.next() {
                match byte {
                    b'%' => {
                        if !bytes.next().is_some_and(|b| b.is_ascii_hexdigit())
                            || !bytes.next().is_some_and(|b| b.is_ascii_hexdigit())
                        {
                            return false;
                        }
                    }
                    b if b.is_ascii_alphanumeric() || b"-._~+*".contains(&b) => {}
                    _ => return false,
                }
            }
            true
        }
    }
}

fn validate_path(method: &Method, path: &str) -> Result<()> {
    if path.len() > MAX_REQUEST_PATH_BYTES || !path.is_ascii() {
        bail!("Docker API endpoint is not allowed");
    }
    let (route, query) = match path.split_once('?') {
        Some((route, query)) => (route, Some(query)),
        None => (path, None),
    };
    let allowed = match *method {
        Method::GET => {
            (route == "/containers/json" && query == Some("all=true"))
                || (route == "/networks" && single_filter_query(query))
                || (query.is_none()
                    && route
                        .strip_prefix("/containers/")
                        .and_then(|tail| tail.strip_suffix("/json"))
                        .is_some_and(safe_name))
                || (query.is_none() && route.strip_prefix("/networks/").is_some_and(safe_name))
        }
        Method::POST if query.is_none() => {
            route == "/networks/create"
                || route
                    .strip_prefix("/networks/")
                    .and_then(|tail| {
                        tail.strip_suffix("/connect")
                            .or_else(|| tail.strip_suffix("/disconnect"))
                    })
                    .is_some_and(full_id)
        }
        Method::DELETE if query.is_none() => route.strip_prefix("/networks/").is_some_and(full_id),
        _ => false,
    };
    if !allowed {
        bail!("Docker API endpoint is not allowed");
    }
    Ok(())
}

fn validate_post_body(path: &str, body: &Value) -> Result<()> {
    let object = body
        .as_object()
        .ok_or_else(|| anyhow!("Docker API request body is not allowed"))?;
    let allowed = if path == "/networks/create" {
        object.len() == 5
            && object
                .get("Name")
                .and_then(Value::as_str)
                .is_some_and(safe_name)
            && object.get("Driver").and_then(Value::as_str) == Some("bridge")
            && object.get("Scope").and_then(Value::as_str) == Some("local")
            && object.get("Internal").and_then(Value::as_bool) == Some(false)
            && object
                .get("Labels")
                .and_then(Value::as_object)
                .is_some_and(|labels| labels.values().all(Value::is_string))
    } else if path.ends_with("/connect") {
        object.len() == 2
            && object
                .get("Container")
                .and_then(Value::as_str)
                .is_some_and(full_id)
            && object
                .get("EndpointConfig")
                .and_then(Value::as_object)
                .is_some_and(|config| {
                    config.len() == 1
                        && config.get("GwPriority").and_then(Value::as_i64) == Some(-1)
                })
    } else if path.ends_with("/disconnect") {
        object.len() == 2
            && object
                .get("Container")
                .and_then(Value::as_str)
                .is_some_and(full_id)
            && object.get("Force").and_then(Value::as_bool) == Some(false)
    } else {
        false
    };
    if !allowed {
        bail!("Docker API request body is not allowed");
    }
    Ok(())
}

fn require_success(response: &Response) -> Result<()> {
    if !response.status().is_success() {
        // Never include the daemon's raw response body, URL or reason phrase.
        bail!("Docker API returned HTTP {}", response.status().as_u16());
    }
    Ok(())
}

async fn read_capped(mut response: Response) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        bail!("Docker API response exceeds the size limit");
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow!("Docker API response read failed"))?
    {
        if chunk.len() > MAX_RESPONSE_BYTES - body.len() {
            bail!("Docker API response exceeds the size limit");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn decode_json(response: Response, allow_empty: bool) -> Result<Value> {
    let body = read_capped(response).await?;
    if allow_empty && body.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&body).map_err(|_| anyhow!("Docker API response is not valid JSON"))
}

fn notify(tx: &mpsc::Sender<()>) -> bool {
    match tx.try_send(()) {
        Ok(()) | Err(mpsc::error::TrySendError::Full(())) => true,
        Err(mpsc::error::TrySendError::Closed(())) => false,
    }
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow_and_update() {
            return;
        }
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

#[derive(Default)]
struct EventLines {
    pending: Vec<u8>,
}

impl EventLines {
    fn push(&mut self, mut chunk: &[u8], tx: &mpsc::Sender<()>) -> Result<bool> {
        while !chunk.is_empty() {
            let newline = chunk.iter().position(|byte| *byte == b'\n');
            let end = newline.unwrap_or(chunk.len());
            if end > MAX_EVENT_LINE_BYTES - self.pending.len() {
                bail!("Docker event record exceeds the size limit");
            }
            self.pending.extend_from_slice(&chunk[..end]);
            if newline.is_some() {
                if !self.finish(tx)? {
                    return Ok(false);
                }
                chunk = &chunk[end + 1..];
            } else {
                break;
            }
        }
        Ok(true)
    }

    fn finish(&mut self, tx: &mpsc::Sender<()>) -> Result<bool> {
        if self.pending.iter().all(u8::is_ascii_whitespace) {
            self.pending.clear();
            return Ok(true);
        }
        let event: Value = serde_json::from_slice(&self.pending)
            .map_err(|_| anyhow!("Docker event record is not valid JSON"))?;
        self.pending.clear();
        if matches!(
            event.get("Type").and_then(Value::as_str),
            Some("container" | "network")
        ) {
            return Ok(notify(tx));
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use std::fs::DirBuilder;
    use std::os::unix::fs::DirBuilderExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{UnixListener, UnixStream};
    use tokio::task::JoinHandle;

    const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OTHER_ID: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const VERSION: &str = r#"{"ApiVersion":"1.58","MinAPIVersion":"1.48"}"#;
    static SOCKET_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct SocketDirectory(PathBuf);

    impl SocketDirectory {
        fn new() -> Self {
            loop {
                let count = SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed);
                let dir =
                    std::env::temp_dir().join(format!("tnc-docker-{}-{count}", std::process::id()));
                match DirBuilder::new().mode(0o700).create(&dir) {
                    Ok(()) => return Self(dir),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("create private mock socket directory: {error}"),
                }
            }
        }

        fn path(&self) -> PathBuf {
            self.0.join("docker.sock")
        }
    }

    impl Drop for SocketDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Debug)]
    struct Request {
        method: String,
        target: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    }

    async fn read_request(stream: &mut UnixStream) -> Request {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 1024];
        let header_end = loop {
            let count = stream.read(&mut buffer).await.expect("read mock request");
            assert!(count > 0, "request ended before headers");
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(pos) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                break pos + 4;
            }
            assert!(bytes.len() < 32 * 1024, "mock request headers too large");
        };
        let head = std::str::from_utf8(&bytes[..header_end]).unwrap();
        let mut lines = head.split("\r\n");
        let mut first = lines.next().unwrap().split_whitespace();
        let method = first.next().unwrap().to_owned();
        let target = first.next().unwrap().to_owned();
        let headers: HashMap<String, String> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
            .collect();
        let length: usize = headers
            .get("content-length")
            .map(|value| value.parse().unwrap())
            .unwrap_or(0);
        while bytes.len() - header_end < length {
            let count = stream.read(&mut buffer).await.expect("read mock body");
            assert!(count > 0, "request ended before body");
            bytes.extend_from_slice(&buffer[..count]);
        }
        Request {
            method,
            target,
            headers,
            body: bytes[header_end..header_end + length].to_vec(),
        }
    }

    fn reply(status: u16, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    struct Server {
        directory: SocketDirectory,
        requests: mpsc::Receiver<Request>,
        task: JoinHandle<()>,
    }

    impl Server {
        fn new(replies: Vec<Vec<u8>>) -> Self {
            let directory = SocketDirectory::new();
            let listener = UnixListener::bind(directory.path()).unwrap();
            let (tx, requests) = mpsc::channel(16);
            let task = tokio::spawn(async move {
                for response in replies {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let request = read_request(&mut stream).await;
                    if tx.send(request).await.is_err() {
                        return;
                    }
                    // Oversize/redirect tests deliberately stop reading early.
                    let _ = stream.write_all(&response).await;
                }
            });
            Self {
                directory,
                requests,
                task,
            }
        }

        fn client(&self) -> Docker {
            Docker::new(&self.directory.path(), Duration::from_secs(2)).unwrap()
        }

        async fn request(&mut self) -> Request {
            timeout(Duration::from_secs(2), self.requests.recv())
                .await
                .expect("mock received request")
                .unwrap()
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    #[tokio::test]
    async fn unix_transport_uses_fixed_authority_and_preserves_filters_and_bodies() {
        let mut server = Server::new(vec![
            reply(200, VERSION),
            reply(200, "[]"),
            reply(201, &format!(r#"{{"Id":"{ID}"}}"#)),
            reply(204, ""),
            reply(204, ""),
        ]);
        let docker = server.client();
        let api = docker.negotiate().await.unwrap();
        assert_eq!(api, "1.56");
        let filters = "/networks?filters=%7B%22label%22%3A%5B%22owner%3Dexample%22%5D%7D";
        assert_eq!(docker.get(&api, filters).await.unwrap(), json!([]));
        let create = json!({
            "Name": "tnc-example",
            "Driver": "bridge",
            "Scope": "local",
            "Internal": false,
            "Labels": {"owner": "example"}
        });
        assert_eq!(
            docker
                .post(&api, "/networks/create", &create)
                .await
                .unwrap(),
            json!({"Id": ID})
        );
        let connect = json!({"Container": OTHER_ID, "EndpointConfig": {"GwPriority": -1}});
        assert_eq!(
            docker
                .post(&api, &format!("/networks/{ID}/connect"), &connect)
                .await
                .unwrap(),
            Value::Null
        );
        docker
            .delete(&api, &format!("/networks/{ID}"))
            .await
            .unwrap();

        let version = server.request().await;
        assert_eq!(
            (version.method.as_str(), version.target.as_str()),
            ("GET", "/version")
        );
        assert_eq!(
            version.headers.get("host").map(String::as_str),
            Some("docker")
        );
        let list = server.request().await;
        assert_eq!(list.target, format!("/v1.56{filters}"));
        let creation = server.request().await;
        assert_eq!(creation.method, "POST");
        assert_eq!(creation.target, "/v1.56/networks/create");
        assert_eq!(
            serde_json::from_slice::<Value>(&creation.body).unwrap(),
            create
        );
        let connection = server.request().await;
        assert_eq!(
            serde_json::from_slice::<Value>(&connection.body).unwrap(),
            connect
        );
        let deletion = server.request().await;
        assert_eq!(deletion.method, "DELETE");
        assert_eq!(deletion.target, format!("/v1.56/networks/{ID}"));
    }

    #[test]
    fn api_negotiation_checks_both_bounds_and_rejects_malformed_versions() {
        assert_eq!(
            choose_api(&json!({"ApiVersion":"1.50","MinAPIVersion":"1.45"})).unwrap(),
            "1.50"
        );
        assert_eq!(
            choose_api(&json!({"ApiVersion":"1.56","MinAPIVersion":"1.56"})).unwrap(),
            "1.56"
        );
        for value in [
            json!({"ApiVersion":"1.47","MinAPIVersion":"1.24"}),
            json!({"ApiVersion":"1.58","MinAPIVersion":"1.57"}),
            json!({"ApiVersion":"1.50","MinAPIVersion":"1.51"}),
            json!({"ApiVersion":"1.56"}),
            json!({"ApiVersion":"1.56/secret","MinAPIVersion":"1.48"}),
            json!({"ApiVersion":"01.56","MinAPIVersion":"1.48"}),
            json!({"ApiVersion":1.56,"MinAPIVersion":"1.48"}),
        ] {
            assert!(choose_api(&value).is_err());
        }
    }

    #[tokio::test]
    async fn server_with_old_api_is_rejected_over_unix_transport() {
        let server = Server::new(vec![reply(
            200,
            r#"{"ApiVersion":"1.47","MinAPIVersion":"1.24"}"#,
        )]);
        let error = server.client().negotiate().await.unwrap_err();
        assert!(error.to_string().contains("no supported API version"));
    }

    #[tokio::test]
    async fn optional_get_means_only_http_404_and_errors_do_not_leak_bodies() {
        let mut server = Server::new(vec![
            reply(404, r#"{"message":"secret-label-token"}"#),
            reply(403, r#"{"message":"secret-label-token"}"#),
            reply(500, r#"{"message":"secret-label-token"}"#),
            reply(200, "not-json-secret-label-token"),
        ]);
        let docker = server.client();
        let path = "/networks/tnc-example";
        assert!(docker.get_optional("1.56", path).await.unwrap().is_none());
        for code in [403, 500] {
            let error = docker.get_optional("1.56", path).await.unwrap_err();
            let formatted = format!("{error:#}");
            assert_eq!(formatted, format!("Docker API returned HTTP {code}"));
            assert!(!formatted.contains("secret-label-token"));
        }
        assert_eq!(
            docker
                .get_optional("1.56", path)
                .await
                .unwrap_err()
                .to_string(),
            "Docker API response is not valid JSON"
        );
        for _ in 0..4 {
            assert_eq!(server.request().await.target, "/v1.56/networks/tnc-example");
        }
    }

    #[tokio::test]
    async fn redirects_are_rejected_without_following_location() {
        let mut response = b"HTTP/1.1 302 Found\r\nLocation: http://elsewhere.invalid/secret-label-token\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
        let mut server = Server::new(vec![std::mem::take(&mut response)]);
        let error = server.client().get("1.56", "/networks").await.unwrap_err();
        assert_eq!(format!("{error:#}"), "Docker API returned HTTP 302");
        assert_eq!(server.request().await.target, "/v1.56/networks");
        assert!(
            timeout(Duration::from_millis(100), server.requests.recv())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn response_cap_checks_declared_and_streamed_lengths() {
        let declared = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            MAX_RESPONSE_BYTES + 1
        )
        .into_bytes();
        let mut streamed =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
        let block = vec![b' '; 1024 * 1024];
        for _ in 0..8 {
            streamed.extend_from_slice(b"100000\r\n");
            streamed.extend_from_slice(&block);
            streamed.extend_from_slice(b"\r\n");
        }
        streamed.extend_from_slice(b"1\r\nx\r\n0\r\n\r\n");
        let server = Server::new(vec![declared, streamed]);
        let docker = server.client();
        for _ in 0..2 {
            assert_eq!(
                docker
                    .get("1.56", "/networks")
                    .await
                    .unwrap_err()
                    .to_string(),
                "Docker API response exceeds the size limit"
            );
        }
    }

    #[tokio::test]
    async fn normal_request_deadline_includes_response_body_reads() {
        let directory = SocketDirectory::new();
        let listener = UnixListener::bind(directory.path()).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut stream).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n[")
                .await
                .unwrap();
            std::future::pending::<()>().await;
        });
        let docker = Docker::new(&directory.path(), Duration::from_millis(100)).unwrap();
        let error = timeout(
            Duration::from_secs(2),
            docker.get(
                "1.56",
                "/networks?filters=%7B%22secret-token%22%3A%5B%5D%7D",
            ),
        )
        .await
        .expect("normal request must honor its deadline")
        .unwrap_err();
        assert_eq!(format!("{error:#}"), "Docker API response read failed");
        server.abort();
    }

    #[test]
    fn endpoint_and_payload_allowlists_reject_other_docker_mutations() {
        for (method, path) in [
            (Method::GET, "http://elsewhere.invalid/networks"),
            (Method::GET, "//elsewhere.invalid/networks"),
            (Method::GET, "/networks/../../containers/json"),
            (Method::GET, "/networks/example#fragment"),
            (Method::GET, "/containers/json?all=true&other=true"),
            (Method::GET, "/networks?filters=%7B%7D&other=true"),
            (Method::GET, "/networks?filters=%G0"),
            (Method::GET, "/containers/example/archive"),
            (Method::POST, "/containers/create"),
            (Method::POST, "/networks/prune"),
            (Method::POST, "/networks/name/connect"),
            (Method::DELETE, "/networks/name"),
        ] {
            assert!(validate_path(&method, path).is_err(), "{method} {path}");
        }
        assert!(validate_path(&Method::GET, "/containers/traefik-1/json").is_ok());
        assert!(validate_path(&Method::GET, "/containers/json?all=true").is_ok());
        assert!(validate_path(&Method::GET, "/networks/tnc-example").is_ok());

        let connect = format!("/networks/{ID}/connect");
        for body in [
            json!({"Container":"name","EndpointConfig":{"GwPriority":-1}}),
            json!({"Container":ID,"EndpointConfig":{"GwPriority":1}}),
            json!({"Container":ID,"EndpointConfig":{"GwPriority":-1,"Aliases":["db"]}}),
            json!({"Container":ID,"EndpointConfig":{"GwPriority":-1},"extra":true}),
        ] {
            assert!(validate_post_body(&connect, &body).is_err());
        }
        let disconnect = format!("/networks/{ID}/disconnect");
        assert!(validate_post_body(&disconnect, &json!({"Container":ID,"Force":true})).is_err());
        assert!(validate_post_body(&disconnect, &json!({"Container":ID,"Force":false})).is_ok());
        assert!(
            validate_post_body(
                "/networks/create",
                &json!({
                    "Name":"tnc-example","Driver":"host","Scope":"local",
                    "Internal":false,"Labels":{}
                })
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn events_survive_request_timeout_coalesce_and_cancel_while_idle() {
        let directory = SocketDirectory::new();
        let listener = UnixListener::bind(directory.path()).unwrap();
        let server = tokio::spawn(async move {
            let (mut version, _) = listener.accept().await.unwrap();
            assert_eq!(read_request(&mut version).await.target, "/version");
            version.write_all(&reply(200, VERSION)).await.unwrap();
            drop(version);
            let (mut events, _) = listener.accept().await.unwrap();
            let request = read_request(&mut events).await;
            assert!(request.target.starts_with("/v1.56/events?filters="));
            let url = reqwest::Url::parse(&format!("{AUTHORITY}{}", request.target)).unwrap();
            let pairs: Vec<_> = url.query_pairs().collect();
            assert_eq!(pairs.len(), 1);
            assert_eq!(pairs[0].0, "filters");
            assert_eq!(pairs[0].1, EVENT_FILTERS);
            events
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            sleep(Duration::from_millis(350)).await;
            // Split a JSON line across HTTP chunks, then add an event burst.
            for part in [
                b"{\"Ty".as_slice(),
                b"pe\":\"container\"}\n{\"Type\":\"network\"}\n".as_slice(),
            ] {
                let header = format!("{:x}\r\n", part.len());
                events.write_all(header.as_bytes()).await.unwrap();
                events.write_all(part).await.unwrap();
                events.write_all(b"\r\n").await.unwrap();
            }
            std::future::pending::<()>().await;
        });
        let docker = Docker::new(&directory.path(), Duration::from_millis(200)).unwrap();
        let (tx, mut rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let watcher = tokio::spawn(async move { docker.watch_events(tx, shutdown_rx).await });
        // Initial stream-open scan, then a real event after the normal request
        // deadline has elapsed. No EOF/reconnect is required to receive it.
        timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(!watcher.is_finished());
        shutdown_tx.send(true).unwrap();
        timeout(Duration::from_millis(500), watcher)
            .await
            .unwrap()
            .unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn shutdown_cancels_a_stalled_version_request() {
        let directory = SocketDirectory::new();
        let listener = UnixListener::bind(directory.path()).unwrap();
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut stream).await;
            let _ = accepted_tx.send(());
            std::future::pending::<()>().await;
        });
        let docker = Docker::new(&directory.path(), Duration::from_secs(30)).unwrap();
        let (tx, _rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let watcher = tokio::spawn(async move { docker.watch_events(tx, shutdown_rx).await });
        timeout(Duration::from_secs(2), accepted_rx)
            .await
            .unwrap()
            .unwrap();
        shutdown_tx.send(true).unwrap();
        timeout(Duration::from_millis(500), watcher)
            .await
            .unwrap()
            .unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn reconnect_renegotiates_the_daemon_api_version() {
        let mut server = Server::new(vec![
            reply(200, r#"{"ApiVersion":"1.50","MinAPIVersion":"1.40"}"#),
            reply(200, "{\"Type\":\"network\"}\n"),
            reply(200, VERSION),
            reply(200, "{\"Type\":\"container\"}\n"),
        ]);
        let docker = server.client();
        let (tx, _rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let watcher = tokio::spawn(async move { docker.watch_events(tx, shutdown_rx).await });
        assert_eq!(server.request().await.target, "/version");
        assert!(server.request().await.target.starts_with("/v1.50/events?"));
        assert_eq!(server.request().await.target, "/version");
        assert!(server.request().await.target.starts_with("/v1.56/events?"));
        shutdown_tx.send(true).unwrap();
        timeout(Duration::from_millis(500), watcher)
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn events_are_bounded_and_bursts_are_coalesced() {
        let (tx, mut rx) = mpsc::channel(1);
        let mut lines = EventLines::default();
        assert!(lines.push(b"{\"Ty", &tx).unwrap());
        assert!(rx.try_recv().is_err());
        assert!(
            lines
                .push(b"pe\":\"container\"}\n{\"Type\":\"network\"}\n", &tx)
                .unwrap()
        );
        assert_eq!(rx.try_recv(), Ok(()));
        assert!(rx.try_recv().is_err());
        assert!(lines.push(b"{\"Type\":\"image\"}\n\n", &tx).unwrap());
        assert!(rx.try_recv().is_err());
        assert!(lines.push(&vec![b' '; MAX_EVENT_LINE_BYTES], &tx).unwrap());
        assert!(lines.push(b"x", &tx).is_err());
    }
}
