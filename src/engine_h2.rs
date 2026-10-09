// Added by HyperSpaces (2026): Verify capped immutable raw ranges over native h2 alongside cancellation and shutdown. Original: durable-streams 0.1.5, Apache-2.0.
//! Private, read-only h2c transport. One Store and the same protocol handlers as h1.
use crate::{
    api::{Body, Method, Req, Resp, SECURITY_HEADERS},
    handlers,
    store::Store,
};
use bytes::Bytes;
use h2::{Reason, SendStream};
use std::{
    future::Future,
    io,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{watch, Semaphore},
    task::JoinSet,
};

pub const STREAMS_PER_CONNECTION: u32 = 1024;
const MAX_CONNECTIONS: usize = 256;
const CHUNK: usize = 16 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn serve(
    store: Arc<Store>,
    listener: TcpListener,
    shutdown: impl Future<Output = ()>,
    grace: Duration,
) {
    let admission = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let (stop, shutdown_connections) = watch::channel(false);
    let mut connections = JoinSet::new();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            result = listener.accept() => match result {
                Ok((socket, _)) => {
                    let Ok(permit) = admission.clone().try_acquire_owned() else { continue };
                    let store = store.clone();
                    let shutdown = shutdown_connections.clone();
                    connections.spawn(async move {
                        let _permit = permit;
                        if let Err(error) = connection(store, socket, shutdown).await {
                            tracing::debug!(%error, "DS h2 connection ended");
                        }
                    });
                }
                Err(error) => {
                    tracing::warn!(%error, "DS h2 accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
    // Stop new sockets and send GOAWAY while existing responses finish. At the
    // shared grace deadline, dropping the JoinSet aborts remaining readers.
    drop(listener);
    let _ = stop.send(true);
    let _ = tokio::time::timeout(grace, async {
        while connections.join_next().await.is_some() {}
    })
    .await;
}

async fn connection(
    store: Arc<Store>,
    socket: TcpStream,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    socket.set_nodelay(true)?;
    let mut builder = h2::server::Builder::new();
    builder
        .max_concurrent_streams(STREAMS_PER_CONNECTION)
        .initial_window_size(CHUNK as u32)
        .initial_connection_window_size(4 * 1024 * 1024)
        .max_send_buffer_size(CHUNK)
        .max_header_list_size(16 * 1024);
    let mut connection = tokio::select! {
        _ = shutdown.changed() => return Ok(()),
        result = tokio::time::timeout(Duration::from_secs(10), builder.handshake(socket)) => {
            result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "h2 handshake"))?
                .map_err(io::Error::other)?
        }
    };
    let mut readers = JoinSet::new();
    let mut draining = false;
    loop {
        tokio::select! {
            _ = shutdown.changed(), if !draining => {
                draining = true;
                connection.graceful_shutdown();
            },
            request = connection.accept() => {
                let Some(request) = request else { return Ok(()) };
                let (request, mut reply) = request.map_err(io::Error::other)?;
                let store = store.clone();
                readers.spawn(async move {
                    // Run the large handler future in its own allocation, released
                    // before a live body parks. No producer/channel per SSE reader.
                    let response = tokio::select! {
                        _ = std::future::poll_fn(|cx| reply.poll_reset(cx)) => return,
                        response = Box::pin(handle(store, request)) => response,
                    };
                    if let Err(error) = write_response(response, &mut reply).await {
                        reply.send_reset(Reason::INTERNAL_ERROR);
                        tracing::debug!(%error, "DS h2 response aborted");
                    }
                });
            },
            Some(_) = readers.join_next(), if !readers.is_empty() => {}
        }
    }
}

async fn handle(store: Arc<Store>, request: http::Request<h2::RecvStream>) -> (Resp, bool) {
    let head = request.method() == http::Method::HEAD;
    if !matches!(
        *request.method(),
        http::Method::GET | http::Method::HEAD | http::Method::OPTIONS
    ) {
        return (Resp::new(405), head);
    }
    // Read methods have no payload. Never collect a user body on this listener.
    if !request.body().is_end_stream()
        || request
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .is_some_and(|value| value != "0")
    {
        return (Resp::new(400), head);
    }
    let mut headers = Vec::with_capacity(request.headers().len());
    for (name, value) in request.headers() {
        let Ok(value) = value.to_str() else {
            return (Resp::new(400), head);
        };
        headers.push((name.as_str().to_owned(), value.to_owned()));
    }
    let req = Req {
        method: Method::parse(request.method().as_str()),
        path: request.uri().path().to_owned(),
        query: request.uri().query().map(str::to_owned),
        headers,
        body: Bytes::new(),
    };
    (handlers::handle(store, req).await, head)
}

async fn write_response(
    (resp, head): (Resp, bool),
    reply: &mut h2::server::SendResponse<Bytes>,
) -> io::Result<()> {
    let no_body = head || crate::http1::status_has_no_body(resp.status);
    let mut response = http::Response::builder()
        .status(resp.status)
        .version(http::Version::HTTP_2);
    for &(name, value) in SECURITY_HEADERS {
        response = response.header(name, value);
    }
    for (name, value) in &resp.headers {
        // The existing SSE handler includes Connection. It is invalid on h2.
        if matches!(
            *name,
            "connection" | "keep-alive" | "proxy-connection" | "transfer-encoding" | "upgrade"
        ) {
            continue;
        }
        response = response.header(*name, value);
    }
    if !resp
        .headers
        .iter()
        .any(|(name, _)| *name == "content-length")
        && !crate::http1::status_has_no_body(resp.status)
    {
        if let Some(len) = resp.body.len() {
            response = response.header("content-length", len);
        }
    }
    let mut send = reply
        .send_response(response.body(()).map_err(io::Error::other)?, no_body)
        .map_err(io::Error::other)?;
    if no_body {
        return Ok(());
    }
    // A canceled reader must drop its SSE source immediately, even when idle.
    // The reset is scoped to this response; its connection remains shared.
    body(resp.body, &mut send).await
}

async fn body(body: Body, send: &mut SendStream<Bytes>) -> io::Result<()> {
    match body {
        Body::Empty => {}
        Body::Full(bytes) => send_bytes(send, bytes).await?,
        Body::Sse(mut source) => loop {
            let chunk = tokio::select! {
                result = std::future::poll_fn(|cx| send.poll_reset(cx)) => {
                    return Err(io::Error::other(format!("reader reset: {result:?}")));
                },
                chunk = source.next_chunk() => chunk,
            };
            let Some(chunk) = chunk? else { break };
            send_bytes(send, chunk).await?;
        },
        Body::Channel(crate::api::StreamBody { mut rx, failed }) => {
            loop {
                let chunk = tokio::select! {
                    result = std::future::poll_fn(|cx| send.poll_reset(cx)) => {
                        return Err(io::Error::other(format!("reader reset: {result:?}")));
                    },
                    chunk = rx.recv() => chunk,
                };
                let Some(chunk) = chunk else { break };
                send_bytes(send, chunk).await?;
            }
            if failed.load(Ordering::Acquire) {
                return Err(io::Error::other("read aborted mid-stream"));
            }
        }
        Body::FileRange {
            segments,
            prefix,
            suffix,
            ..
        } => {
            send_bytes(send, Bytes::from_static(prefix)).await?;
            for segment in segments {
                let mut start = segment.file_start;
                let end = start + segment.len;
                while start < end {
                    // One bounded positioned read; never materialize a historical log.
                    let len = (end - start).min(CHUNK as u64) as usize;
                    let file = segment.file.clone();
                    let bytes = tokio::task::spawn_blocking(move || {
                        use std::os::unix::fs::FileExt;
                        let mut bytes = vec![0; len];
                        file.read_exact_at(&mut bytes, start)?;
                        Ok::<_, io::Error>(Bytes::from(bytes))
                    })
                    .await
                    .map_err(io::Error::other)??;
                    send_bytes(send, bytes).await?;
                    start += len as u64;
                }
            }
            send_bytes(send, Bytes::from_static(suffix)).await?;
        }
    }
    send.send_data(Bytes::new(), true).map_err(io::Error::other)
}

async fn send_bytes(send: &mut SendStream<Bytes>, mut bytes: Bytes) -> io::Result<()> {
    while !bytes.is_empty() {
        send.reserve_capacity(bytes.len().min(CHUNK));
        let capacity = tokio::time::timeout(
            WRITE_TIMEOUT,
            std::future::poll_fn(|cx| send.poll_capacity(cx)),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "slow h2 reader"))?
        .ok_or_else(|| io::Error::other("h2 response closed"))?
        .map_err(io::Error::other)?;
        if capacity == 0 {
            continue;
        }
        let len = capacity.min(bytes.len()).min(CHUNK);
        send.send_data(bytes.split_to(len), false)
            .map_err(io::Error::other)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn h1(
        address: std::net::SocketAddr,
        method: &str,
        path: &str,
        payload: &[u8],
    ) -> Vec<u8> {
        h1_content(address, method, path, payload, "application/octet-stream").await
    }

    async fn h1_content(address: std::net::SocketAddr, method: &str, path: &str,
        payload: &[u8], content_type: &str) -> Vec<u8> {
        let mut socket = TcpStream::connect(address).await.unwrap();
        socket.write_all(format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n", payload.len()).as_bytes()).await.unwrap();
        socket.write_all(payload).await.unwrap();
        let mut response = Vec::new();
        socket.read_to_end(&mut response).await.unwrap();
        response
    }

    async fn read(
        sender: &h2::client::SendRequest<Bytes>,
        method: &str,
        path: &str,
    ) -> http::Response<h2::RecvStream> {
        let mut sender = sender.clone().ready().await.unwrap();
        let request = http::Request::builder()
            .method(method)
            .version(http::Version::HTTP_2)
            .uri(format!("http://localhost{path}"))
            .body(())
            .unwrap();
        let (response, _) = sender.send_request(request, true).unwrap();
        response.await.unwrap()
    }

    async fn collect(mut response: http::Response<h2::RecvStream>) -> Vec<u8> {
        let mut bytes = Vec::new();
        while let Some(chunk) = response.body_mut().data().await {
            let chunk = chunk.unwrap();
            response
                .body_mut()
                .flow_control()
                .release_capacity(chunk.len())
                .unwrap();
            bytes.extend_from_slice(&chunk);
        }
        bytes
    }

    #[tokio::test]
    async fn raw_range_crosses_sealed_tail_boundary_and_rejects_damaged_storage() {
        let _guard = crate::handlers::test_support::DurabilityGuard::memory();
        let dir = std::env::temp_dir().join(format!("ds-raw-tier-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let store = Arc::new(Store::new_with_tier(dir.clone(), crate::tier::TierConfig {
            kind: crate::tier::TierKind::Local, segment_bytes: 256 * 1024,
            compact_bytes: 256 * 1024, local_dir: Some(dir.join("cold")), ..Default::default()
        }).unwrap());
        let h1_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let h1_address = h1_listener.local_addr().unwrap();
        let h1_server = tokio::spawn(crate::engine_raw::serve(store.clone(), h1_listener));
        let h2_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let h2_address = h2_listener.local_addr().unwrap();
        let (stop, shutdown) = tokio::sync::oneshot::channel();
        let h2_server = tokio::spawn(serve(store.clone(), h2_listener,
            async { let _ = shutdown.await; }, Duration::from_secs(2)));
        let payload: Vec<u8> = (0..256 * 1024 + 1000).map(|index| (index % 251) as u8).collect();
        assert!(h1(h1_address, "PUT", "/v1/stream/tier-range", &payload).await.starts_with(b"HTTP/1.1 201"));
        let state = store.get("/v1/stream/tier-range").unwrap();
        store.maybe_seal(&state).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if !state.tier.manifest.lock().unwrap().offloading { break; }
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        let sealed = state.tier.manifest.lock().unwrap().sealed_offset;
        assert_eq!(sealed, 256 * 1024);
        let key = match &state.tier.manifest.lock().unwrap().segments[0].placement {
            crate::tier::Placement::Remote(key) => key.clone(), _ => panic!("segment not offloaded"),
        };
        let (sender, driver) = h2::client::handshake(TcpStream::connect(h2_address).await.unwrap()).await.unwrap();
        let driver = tokio::spawn(driver);
        let response = read(&sender, "GET", "/v1/stream/tier-range?range=bytes=-1048576").await;
        assert_eq!(response.status(), 206);
        let cut = response.headers()["stream-read-cut"].to_str().unwrap().to_string();
        assert_eq!(collect(response).await, payload);
        let path = format!("/v1/stream/tier-range?range=bytes={}-{}&cut={cut}", sealed - 10, sealed + 9);
        assert_eq!(collect(read(&sender, "GET", &path).await).await, payload[sealed as usize - 10..sealed as usize + 10]);

        // The mixed range cannot be served by the resident tail cache. A short
        // backing file must reset the response rather than claim complete bytes.
        std::fs::OpenOptions::new().write(true).open(&state.file_path).unwrap().set_len(0).unwrap();
        let mut truncated = read(&sender, "GET", &path).await;
        assert_eq!(truncated.status(), 206);
        let result = tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(chunk) = truncated.body_mut().data().await {
                match chunk {
                    Err(error) => return Some(error.reason()),
                    Ok(bytes) => truncated.body_mut().flow_control().release_capacity(bytes.len()).unwrap(),
                }
            }
            None
        }).await.unwrap();
        assert_eq!(result, Some(Some(Reason::INTERNAL_ERROR)));
        std::fs::remove_file(dir.join("cold").join(key.replace('/', "_"))).unwrap();
        let mut missing = read(&sender, "GET", &path).await;
        assert_eq!(missing.status(), 206);
        assert_eq!(tokio::time::timeout(Duration::from_secs(2), missing.body_mut().data()).await.unwrap()
            .expect("missing segment ended cleanly").unwrap_err().reason(), Some(Reason::INTERNAL_ERROR));
        assert_eq!(read(&sender, "GET", "/health").await.status(), 200);
        drop(sender); driver.abort(); h1_server.abort(); let _ = stop.send(());
        h2_server.await.unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn h1_writes_and_h2_reads_share_store_and_cancel_only_one_reader() {
        let _guard = crate::handlers::test_support::DurabilityGuard::memory();
        let dir = std::env::temp_dir().join(format!(
            "ds-h2-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = Arc::new(
            Store::new_with_tier(dir.clone(), crate::tier::TierConfig::default()).unwrap(),
        );
        let h1_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let h1_address = h1_listener.local_addr().unwrap();
        let h2_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let h2_address = h2_listener.local_addr().unwrap();
        let h1_server = tokio::spawn(crate::engine_raw::serve(store.clone(), h1_listener));
        let (stop, shutdown) = tokio::sync::oneshot::channel();
        let h2_server = tokio::spawn(serve(
            store.clone(),
            h2_listener,
            async {
                let _ = shutdown.await;
            },
            Duration::from_secs(2),
        ));
        // Larger than transport windows: file ranges must make bounded progress.
        let payload = vec![b'a'; 1024 * 1024];
        assert!(h1(h1_address, "PUT", "/v1/stream/h2", &payload)
            .await
            .starts_with(b"HTTP/1.1 201"));
        let (sender, driver) = h2::client::handshake(TcpStream::connect(h2_address).await.unwrap())
            .await
            .unwrap();
        let driver = tokio::spawn(driver);
        let response = read(&sender, "GET", "/v1/stream/h2?offset=-1").await;
        assert_eq!(response.version(), http::Version::HTTP_2);
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        let tail = response.headers()["stream-next-offset"]
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(collect(response).await, payload);
        let head = read(&sender, "HEAD", "/v1/stream/h2").await;
        assert_eq!(head.headers()["stream-next-offset"], tail);
        assert!(collect(head).await.is_empty());
        // Raw JSON representation retains separators; no wrapper, decode or scan.
        let raw_path = "/v1/stream/raw-history";
        let initial = r#"[{"label":"quoted \"} 🧭"},{"label":"two"}]"#;
        assert!(h1_content(h1_address, "PUT", raw_path, initial.as_bytes(), "application/json")
            .await.starts_with(b"HTTP/1.1 201"));
        let expected = r#"{"label":"quoted \"} 🧭"},{"label":"two"},"#.as_bytes();
        let raw = read(&sender, "GET", "/v1/stream/raw-history?range=bytes=-1048576").await;
        assert_eq!(raw.status(), 206);
        assert_eq!(raw.headers()["content-type"], "application/octet-stream");
        assert_eq!(raw.headers()["content-range"], format!("bytes 0-{}/{}", expected.len() - 1, expected.len()));
        let cut = raw.headers()["stream-read-cut"].to_str().unwrap().to_string();
        assert_eq!(collect(raw).await, expected);
        assert!(h1_content(h1_address, "POST", raw_path, br#"{"label":"later"}"#, "application/json")
            .await.starts_with(b"HTTP/1.1 204"));
        let bounded = read(&sender, "GET", &format!("{raw_path}?range=bytes=5-10&cut={cut}")).await;
        assert_eq!(bounded.status(), 206);
        assert_eq!(bounded.headers()["stream-read-cut"], cut);
        assert_eq!(collect(bounded).await, &expected[5..11]);
        assert_eq!(read(&sender, "GET", &format!("{raw_path}?range=bytes=-1048577")).await.status(), 416);
        assert_eq!(read(&sender, "GET", &format!("{raw_path}?range=bytes=-16&offset=-1")).await.status(), 400);
        assert_eq!(read(&sender, "GET", &format!("{raw_path}?range=bytes=-16&cut=0:1")).await.status(), 409);
        assert!(h1(h1_address, "DELETE", raw_path, b"").await.starts_with(b"HTTP/1.1 204"));
        assert!(h1_content(h1_address, "PUT", raw_path, b"[]", "application/json").await.starts_with(b"HTTP/1.1 201"));
        assert_eq!(read(&sender, "GET", &format!("{raw_path}?range=bytes=-16&cut={cut}")).await.status(), 409);
        let empty = read(&sender, "GET", &format!("{raw_path}?range=bytes=-16")).await;
        assert_eq!(empty.status(), 204);
        assert_eq!(empty.headers()["content-range"], "bytes */0");
        assert!(collect(empty).await.is_empty());

        assert_eq!(read(&sender, "PUT", "/v1/stream/h2").await.status(), 405);
        let path = format!("/v1/stream/h2?offset={tail}&live=sse");
        let removed = read(&sender, "GET", &path).await;
        let mut staying = read(&sender, "GET", &path).await;
        assert!(!staying.headers().contains_key("connection"));
        // Drain initial up-to-date control, then cancel the other logical read.
        let initial = staying.body_mut().data().await.unwrap().unwrap();
        staying
            .body_mut()
            .flow_control()
            .release_capacity(initial.len())
            .unwrap();
        drop(removed);
        assert!(h1(h1_address, "POST", "/v1/stream/h2", b"survivor")
            .await
            .starts_with(b"HTTP/1.1 204"));
        let next = tokio::time::timeout(Duration::from_secs(2), staying.body_mut().data())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(std::str::from_utf8(&next).unwrap().contains("c3Vydml2b3I="));
        staying
            .body_mut()
            .flow_control()
            .release_capacity(next.len())
            .unwrap();
        assert_eq!(read(&sender, "GET", "/health").await.status(), 200);
        drop(staying);
        // A damaged historical range misses the resident tail cache. The
        // existing SSE source must report its real file I/O error as RST_STREAM,
        // preserving the last offset and this shared connection's other reads.
        let state = store.get("/v1/stream/h2").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&state.file_path)
            .unwrap()
            .set_len(0)
            .unwrap();
        let mut broken = read(&sender, "GET", "/v1/stream/h2?offset=-1&live=sse").await;
        assert_eq!(broken.status(), 200);
        let error = tokio::time::timeout(Duration::from_secs(2), broken.body_mut().data())
            .await
            .unwrap()
            .expect("failed SSE ended cleanly")
            .unwrap_err();
        assert_eq!(error.reason(), Some(Reason::INTERNAL_ERROR));
        assert_eq!(read(&sender, "GET", "/health").await.status(), 200);
        drop(broken);
        #[cfg(target_os = "linux")]
        {
            // The same damaged root range takes the real Linux reactor path.
            // A file failure must close without a successful chunk terminator.
            let response = tokio::time::timeout(
                Duration::from_secs(2),
                h1(h1_address, "GET", "/v1/stream/h2?offset=-1&live=sse", b""),
            )
            .await
            .unwrap();
            assert!(
                !response.ends_with(b"0\r\n\r\n"),
                "reactor hid file error as clean EOF"
            );
            assert!(h1(h1_address, "GET", "/health", b"")
                .await
                .starts_with(b"HTTP/1.1 200"));
        }
        // An unread finite response exceeds the flow-control window. Shutdown
        // must let it finish, then bound the remaining idle live reader's drain.
        assert!(h1(h1_address, "PUT", "/v1/stream/drain", &payload)
            .await
            .starts_with(b"HTTP/1.1 201"));
        let mut live = read(&sender, "GET", "/v1/stream/drain?offset=now&live=sse").await;
        let initial = live.body_mut().data().await.unwrap().unwrap();
        live.body_mut()
            .flow_control()
            .release_capacity(initial.len())
            .unwrap();
        // Establish the idle reader before the unread finite body consumes the
        // client connection window, so setup cannot wait for its own drain.
        let finite = read(&sender, "GET", "/v1/stream/drain?offset=-1").await;
        let started = tokio::time::Instant::now();
        stop.send(()).unwrap();
        assert_eq!(collect(finite).await, payload);
        assert!(
            !h2_server.is_finished(),
            "idle reader skipped the drain grace"
        );
        tokio::time::timeout(Duration::from_secs(3), h2_server)
            .await
            .unwrap()
            .unwrap();
        assert!(started.elapsed() >= Duration::from_secs(2));
        let ended = tokio::time::timeout(Duration::from_secs(1), live.body_mut().data())
            .await
            .unwrap()
            .expect("forced drain ended cleanly")
            .unwrap_err();
        assert!(ended.is_io() || ended.reason().is_some());
        h1_server.abort();
        driver.abort();
        let _ = h1_server.await;
        let _ = driver.await;
        std::fs::remove_dir_all(dir).unwrap();
    }
}
