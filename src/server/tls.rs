//! TLS certificate/key loading and a custom `axum::serve::Listener` that wraps a
//! TCP listener with a `tokio-rustls` acceptor.

use std::fmt::Write;
use std::fs::File;
use std::io::{self, BufReader};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};
use tokio_rustls::TlsAcceptor;

/// TLS certificate and private key file paths (PEM format).
///
/// Logs the SHA-256 fingerprint of each certificate on load.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub cert_path: String,
    pub key_path: String,
}

impl TlsConfig {
    /// Create a new `TlsConfig` with paths to the certificate and key PEM files.
    pub fn new(cert_path: String, key_path: String) -> Self {
        Self {
            cert_path,
            key_path,
        }
    }

    /// Load and parse the certificate and key files, returning a `rustls::ServerConfig`
    /// with ALPN protocols `h2` and `http/1.1`. Logs certificate fingerprints.
    pub fn load(&self) -> io::Result<rustls::ServerConfig> {
        let cert_file = match File::open(&self.cert_path) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!(path = %self.cert_path, error = %e, "failed to open TLS certificate file");
                return Err(e);
            }
        };
        let cert_file = &mut BufReader::new(cert_file);

        let certs: Vec<CertificateDer> = match rustls_pemfile::certs(cert_file).collect() {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(path = %self.cert_path, error = %e, "failed to parse TLS certificate");
                return Err(io::Error::new(io::ErrorKind::InvalidData, e));
            }
        };

        if certs.is_empty() {
            tracing::error!(path = %self.cert_path, "no certificates found in TLS certificate file");
            let e = "no certificates found";
            return Err(io::Error::new(io::ErrorKind::InvalidData, e));
        }

        for (i, cert) in certs.iter().enumerate() {
            let fingerprint = Sha256::digest(cert.as_ref());
            let mut hex = String::with_capacity(fingerprint.len() * 3);
            for (j, b) in fingerprint.iter().enumerate() {
                if j > 0 {
                    hex.push(':');
                }
                write!(&mut hex, "{b:02X}").unwrap();
            }
            tracing::info!(%self.cert_path, index = i, fingerprint = %hex, "TLS certificate loaded");
        }

        let key_file = match File::open(&self.key_path) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!(path = %self.key_path, error = %e, "failed to open TLS private key file");
                return Err(e);
            }
        };
        let key_file = &mut BufReader::new(key_file);

        let key = match rustls_pemfile::private_key(key_file) {
            Ok(Some(k)) => k,
            Ok(None) => {
                tracing::error!(path = %self.key_path, "no private key found in TLS key file");
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "no private key found",
                ));
            }
            Err(e) => {
                tracing::error!(path = %self.key_path, error = %e, "failed to parse TLS private key");
                return Err(io::Error::new(io::ErrorKind::InvalidData, e));
            }
        };

        let key = match key {
            PrivateKeyDer::Pkcs8(k) => k.into(),
            PrivateKeyDer::Sec1(k) => k.into(),
            PrivateKeyDer::Pkcs1(k) => k.into(),
            _ => {
                tracing::error!(path = %self.key_path, "unsupported private key format");
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsupported private key format",
                ));
            }
        };

        match rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
        {
            Ok(mut config) => {
                config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
                Ok(config)
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to build TLS server config");
                Err(io::Error::new(io::ErrorKind::InvalidData, e))
            }
        }
    }
}

/// A TLS listener implementing `axum::serve::Listener`.
///
/// Wraps a `tokio::net::TcpListener` with a `tokio_rustls::TlsAcceptor`.
/// Handshake failures are logged and retried; accept errors are logged with a 1-second backoff.
pub struct TlsListener {
    inner: tokio::net::TcpListener,
    acceptor: TlsAcceptor,
}

impl TlsListener {
    /// Bind to `addr` and wrap the TCP listener with a TLS acceptor.
    pub async fn bind(addr: SocketAddr, ls_config: rustls::ServerConfig) -> io::Result<Self> {
        let inner = tokio::net::TcpListener::bind(addr).await?;
        let acceptor = TlsAcceptor::from(Arc::new(ls_config));
        Ok(Self { inner, acceptor })
    }
}

impl axum::serve::Listener for TlsListener {
    type Io = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, addr) = match self.inner.accept().await {
                Ok(tup) => tup,
                Err(e) => {
                    tracing::error!("accept error: {e}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            match self.acceptor.accept(stream).await {
                Ok(tls_stream) => return (tls_stream, addr),
                Err(e) => {
                    tracing::debug!(%addr, error = %e, "TLS handshake failed");
                    continue;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
}
