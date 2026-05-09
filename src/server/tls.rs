use std::io::BufReader;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use std::{fs::File, io};

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};
use tokio_rustls::TlsAcceptor;

#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub cert_path: String,
    pub key_path: String,
}

impl TlsConfig {
    pub fn new(cert_path: String, key_path: String) -> Self {
        Self {
            cert_path,
            key_path,
        }
    }

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
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "no certificates found",
            ));
        }

        for (i, cert) in certs.iter().enumerate() {
            let fingerprint = Sha256::digest(cert.as_ref());
            let hex = fingerprint
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(":");
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

pub struct TlsListener {
    inner: tokio::net::TcpListener,
    acceptor: TlsAcceptor,
}

impl TlsListener {
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
