use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug)]
pub struct CapturedRequest {
    pub head: String,
    pub body: Vec<u8>,
}

pub fn response(status: &str, body: &str, retry_after: Option<&str>) -> String {
    let retry_header = retry_after
        .map(|value| format!("Retry-After: {value}\r\n"))
        .unwrap_or_default();
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{retry_header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

pub fn response_with_content_length(status: &str, content_length: usize) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
    )
}

pub async fn server(
    responses: Vec<String>,
) -> (
    String,
    Arc<Mutex<Vec<CapturedRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured_requests = Arc::clone(&requests);
    let handle = tokio::spawn(async move {
        for response in responses {
            let (mut stream, _) = listener.accept().await.expect("accept test request");
            let request = read_request(&mut stream).await.expect("read test request");
            captured_requests
                .lock()
                .expect("capture lock")
                .push(request);
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write test response");
        }
    });
    (format!("http://{address}"), requests, handle)
}

pub async fn close_after_request_server() -> (
    String,
    Arc<Mutex<Vec<CapturedRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured_requests = Arc::clone(&requests);
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept test request");
        let request = read_request(&mut stream).await.expect("read test request");
        captured_requests
            .lock()
            .expect("capture lock")
            .push(request);
        drop(stream);

        if let Ok(Ok((mut stream, _))) =
            tokio::time::timeout(std::time::Duration::from_millis(400), listener.accept()).await
        {
            let request = read_request(&mut stream)
                .await
                .expect("read retried request");
            captured_requests
                .lock()
                .expect("capture lock")
                .push(request);
        }
    });
    (format!("http://{address}"), requests, handle)
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> std::io::Result<CapturedRequest> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
    };

    let head = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    let content_length = head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or_default();
    while bytes.len() < header_end + content_length {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Ok(CapturedRequest {
        head,
        body: bytes[header_end..header_end + content_length].to_vec(),
    })
}

pub fn header<'a>(request: &'a CapturedRequest, name: &str) -> Option<&'a str> {
    request
        .head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
}
