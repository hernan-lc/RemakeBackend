//! ClamAV INSTREAM: send exactly the verified copy and fail closed on all ambiguity.
use sha2::{Digest, Sha256};
use std::{net::SocketAddr, path::Path, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

#[derive(Clone)]
pub struct ClamAv {
    addr: SocketAddr,
    timeout: Duration,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Scan {
    Clean,
    Detected,
    Unavailable,
}
impl ClamAv {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            timeout: Duration::from_secs(120),
        }
    }
    pub async fn scan(&self, path: &Path, expected_size: u64, expected_digest: &str) -> Scan {
        if !self.addr.ip().is_loopback()
            || self.addr.port() == 0
            || expected_size == 0
            || expected_size > cloud_verifier::MAX_COMPRESSED_BYTES
            || expected_digest.len() != 64
        {
            return Scan::Unavailable;
        }
        match tokio::time::timeout(self.timeout, self.run(path, expected_size, expected_digest))
            .await
        {
            Ok(Some(result)) => result,
            _ => Scan::Unavailable,
        }
    }
    async fn run(&self, path: &Path, expected_size: u64, expected_digest: &str) -> Option<Scan> {
        let mut file = tokio::fs::File::open(path).await.ok()?;
        if !file.metadata().await.ok()?.is_file() {
            return None;
        }
        let mut socket = TcpStream::connect(self.addr).await.ok()?;
        socket.write_all(b"zINSTREAM\0").await.ok()?;
        let mut hash = Sha256::new();
        let mut count = 0_u64;
        let mut buf = vec![0_u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf).await.ok()?;
            if n == 0 {
                break;
            }
            count = count.checked_add(n as u64)?;
            if count > expected_size {
                return None;
            }
            hash.update(&buf[..n]);
            socket.write_all(&(n as u32).to_be_bytes()).await.ok()?;
            socket.write_all(&buf[..n]).await.ok()?;
        }
        if count != expected_size || format!("{:x}", hash.finalize()) != expected_digest {
            return None;
        }
        socket.write_all(&0_u32.to_be_bytes()).await.ok()?;
        let mut reply = Vec::new();
        socket.take(1025).read_to_end(&mut reply).await.ok()?;
        if reply == b"stream: OK\0" {
            return Some(Scan::Clean);
        }
        if reply.starts_with(b"stream: ")
            && reply.ends_with(b" FOUND\0")
            && reply.len() <= 1024
            && reply[8..reply.len() - 7]
                .iter()
                .all(|c| c.is_ascii_graphic() || *c == b' ')
            && reply.len() > 15
        {
            return Some(Scan::Detected);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    async fn fixture(
        response: &'static [u8],
        expected: Arc<Vec<u8>>,
    ) -> (ClamAv, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let scanner = ClamAv {
            addr: listener.local_addr().unwrap(),
            timeout: Duration::from_secs(2),
        };
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut command = [0u8; 10];
            socket.read_exact(&mut command).await.unwrap();
            assert_eq!(&command, b"zINSTREAM\0");
            let mut bytes = Vec::new();
            loop {
                let mut header = [0u8; 4];
                if socket.read_exact(&mut header).await.is_err() {
                    return;
                }
                let n = u32::from_be_bytes(header) as usize;
                if n == 0 {
                    break;
                }
                assert!(n <= 65536);
                let offset = bytes.len();
                bytes.resize(offset + n, 0);
                socket.read_exact(&mut bytes[offset..]).await.unwrap();
            }
            assert_eq!(&bytes, expected.as_ref());
            socket.write_all(response).await.unwrap();
        });
        (scanner, handle)
    }
    #[tokio::test]
    async fn only_exact_clean_or_detected_response_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifact");
        let bytes = Arc::new(vec![42u8; 70000]);
        std::fs::write(&path, bytes.as_ref()).unwrap();
        let digest = format!("{:x}", Sha256::digest(bytes.as_ref()));
        for (reply, expected) in [
            (b"stream: OK\0".as_slice(), Scan::Clean),
            (b"stream: test FOUND\0", Scan::Detected),
            (b"stream: ERROR\0", Scan::Unavailable),
            (b"stream: OK\0junk", Scan::Unavailable),
            (b"stream:  FOUND\0", Scan::Unavailable),
        ] {
            let (scanner, server) = fixture(reply, bytes.clone()).await;
            assert_eq!(
                scanner.scan(&path, bytes.len() as u64, &digest).await,
                expected
            );
            server.await.unwrap();
        }
        let (scanner, server) = fixture(b"stream: OK\0", bytes.clone()).await;
        assert_eq!(
            scanner
                .scan(&path, bytes.len() as u64, &"0".repeat(64))
                .await,
            Scan::Unavailable
        );
        server.await.unwrap();
        let scanner = ClamAv::new("192.0.2.1:3310".parse().unwrap());
        assert_eq!(
            scanner.scan(&path, bytes.len() as u64, &digest).await,
            Scan::Unavailable
        );
    }
    #[tokio::test]
    async fn unresponsive_scanner_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifact");
        std::fs::write(&path, b"hello").unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let scanner = ClamAv {
            addr: listener.local_addr().unwrap(),
            timeout: Duration::from_millis(100),
        };
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        assert_eq!(
            scanner
                .scan(&path, 5, &format!("{:x}", Sha256::digest(b"hello")))
                .await,
            Scan::Unavailable
        );
        server.abort();
    }
}
