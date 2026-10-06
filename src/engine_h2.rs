//! Private, read-only h2c transport. One Store and the same protocol handlers as h1.
use crate::{
    api::{Body, Method, Req, Resp, SECURITY_HEADERS},
    handlers,
    store::Store,
};
use bytes::Bytes;
use h2::{Reason, SendStream};
use std::{
    io,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    task::JoinSet,
};

pub const STREAMS_PER_CONNECTION: u32 = 1024;
const MAX_CONNECTIONS: usize = 256;
const CHUNK: usize = 16 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn serve(store: Arc<Store>, listener: TcpListener) {
    let admission = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    // Dropping this accept loop also aborts its connections and their readers.
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            result = listener.accept() => match result {
                Ok((socket, _)) => {
                    let Ok(permit) = admission.clone().try_acquire_owned() else { continue };
                    let store = store.clone();
                    connections.spawn(async move {
                        let _permit = permit;
                        if let Err(error) = connection(store, socket).await {
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
}

async fn connection(store: Arc<Store>, socket: TcpStream) -> io::Result<()> {
    socket.set_nodelay(true)?;
    let mut builder = h2::server::Builder::new();
    builder
        .max_concurrent_streams(STREAMS_PER_CONNECTION)
        .initial_window_size(CHUNK as u32)
        .initial_connection_window_size(4 * 1024 * 1024)
        .max_send_buffer_size(CHUNK)
        .max_header_list_size(16 * 1024);
    let mut connection = tokio::time::timeout(Duration::from_secs(10), builder.handshake(socket))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "h2 handshake"))?
        .map_err(io::Error::other)?;
    let mut readers = JoinSet::new();
    loop {
        tokio::select! {
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
        let mut socket = TcpStream::connect(address).await.unwrap();
        socket.write_all(format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n", payload.len()).as_bytes()).await.unwrap();
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
        let h2_server = tokio::spawn(serve(store.clone(), h2_listener));
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
        h1_server.abort();
        h2_server.abort();
        driver.abort();
        let _ = h1_server.await;
        let _ = h2_server.await;
        let _ = driver.await;
        std::fs::remove_dir_all(dir).unwrap();
    }
}
