use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::error::{PrivoxyError, PrivoxyResult};

#[cfg(feature = "openssl-ssl")]
use openssl::ssl::{SslConnector as OpensslSslConnector, SslMethod};
#[cfg(feature = "openssl-ssl")]
use tokio_openssl::SslStream as TokioOpensslSslStream;

#[cfg(feature = "native-tls")]
use native_tls::TlsConnector as NativeTlsConnector;
#[cfg(feature = "native-tls")]
use tokio_native_tls::TlsStream as TokioNativeTlsStream;

#[cfg(feature = "rustls")]
use rustls::ClientConfig as RustlsClientConfig;
#[cfg(feature = "rustls")]
use rustls::pki_types::ServerName;
#[cfg(feature = "rustls")]
use tokio_rustls::TlsConnector as RustlsConnector;

#[cfg(feature = "wolfssl")]
use wolfssl::SslConnector as WolfsslSslConnector;

pub enum SslConnector {
    #[cfg(feature = "openssl-ssl")]
    Openssl(OpensslSslConnector),
    
    #[cfg(feature = "native-tls")]
    NativeTls(NativeTlsConnector),
    
    #[cfg(feature = "rustls")]
    Rustls(Arc<RustlsClientConfig>),
    
    #[cfg(feature = "wolfssl")]
    Wolfssl(WolfsslSslConnector),
    
    #[cfg(not(any(feature = "openssl-ssl", feature = "native-tls", feature = "rustls", feature = "wolfssl")))]
    None,
}

pub enum SslStream<S> {
    #[cfg(feature = "openssl-ssl")]
    Openssl(TokioOpensslSslStream<S>),
    
    #[cfg(feature = "native-tls")]
    NativeTls(TokioNativeTlsStream<S>),
    
    #[cfg(feature = "rustls")]
    Rustls(tokio_rustls::client::TlsStream<S>),
    
    #[cfg(feature = "wolfssl")]
    Wolfssl(wolfssl::SslStream<S>),
    
    #[cfg(not(any(feature = "openssl-ssl", feature = "native-tls", feature = "rustls", feature = "wolfssl")))]
    None(S),
}

impl<S> AsyncRead for SslStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut *self {
            #[cfg(feature = "openssl-ssl")]
            SslStream::Openssl(stream) => Pin::new(stream).poll_read(cx, buf),
            
            #[cfg(feature = "native-tls")]
            SslStream::NativeTls(stream) => Pin::new(stream).poll_read(cx, buf),
            
            #[cfg(feature = "rustls")]
            SslStream::Rustls(stream) => Pin::new(stream).poll_read(cx, buf),
            
            #[cfg(feature = "wolfssl")]
            SslStream::Wolfssl(stream) => Pin::new(stream).poll_read(cx, buf),
            
            #[cfg(not(any(feature = "openssl-ssl", feature = "native-tls", feature = "rustls", feature = "wolfssl")))]
            SslStream::None(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl<S> AsyncWrite for SslStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        match &mut *self {
            #[cfg(feature = "openssl-ssl")]
            SslStream::Openssl(stream) => Pin::new(stream).poll_write(cx, buf),
            
            #[cfg(feature = "native-tls")]
            SslStream::NativeTls(stream) => Pin::new(stream).poll_write(cx, buf),
            
            #[cfg(feature = "rustls")]
            SslStream::Rustls(stream) => Pin::new(stream).poll_write(cx, buf),
            
            #[cfg(feature = "wolfssl")]
            SslStream::Wolfssl(stream) => Pin::new(stream).poll_write(cx, buf),
            
            #[cfg(not(any(feature = "openssl-ssl", feature = "native-tls", feature = "rustls", feature = "wolfssl")))]
            SslStream::None(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        match &mut *self {
            #[cfg(feature = "openssl-ssl")]
            SslStream::Openssl(stream) => Pin::new(stream).poll_flush(cx),
            
            #[cfg(feature = "native-tls")]
            SslStream::NativeTls(stream) => Pin::new(stream).poll_flush(cx),
            
            #[cfg(feature = "rustls")]
            SslStream::Rustls(stream) => Pin::new(stream).poll_flush(cx),
            
            #[cfg(feature = "wolfssl")]
            SslStream::Wolfssl(stream) => Pin::new(stream).poll_flush(cx),
            
            #[cfg(not(any(feature = "openssl-ssl", feature = "native-tls", feature = "rustls", feature = "wolfssl")))]
            SslStream::None(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        match &mut *self {
            #[cfg(feature = "openssl-ssl")]
            SslStream::Openssl(stream) => Pin::new(stream).poll_shutdown(cx),
            
            #[cfg(feature = "native-tls")]
            SslStream::NativeTls(stream) => Pin::new(stream).poll_shutdown(cx),
            
            #[cfg(feature = "rustls")]
            SslStream::Rustls(stream) => Pin::new(stream).poll_shutdown(cx),
            
            #[cfg(feature = "wolfssl")]
            SslStream::Wolfssl(stream) => Pin::new(stream).poll_shutdown(cx),
            
            #[cfg(not(any(feature = "openssl-ssl", feature = "native-tls", feature = "rustls", feature = "wolfssl")))]
            SslStream::None(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

impl SslConnector {
    pub fn new() -> PrivoxyResult<Self> {
        #[cfg(feature = "openssl-ssl")]
        {
            let connector = OpensslSslConnector::builder(SslMethod::tls())
                .map_err(|e| PrivoxyError::Ssl(format!("Failed to create SSL connector: {}", e)))?
                .build();
            Ok(SslConnector::Openssl(connector))
        }
        
        #[cfg(all(feature = "native-tls", not(feature = "openssl-ssl")))]
        {
            let connector = NativeTlsConnector::new()
                .map_err(|e| PrivoxyError::Ssl(format!("Failed to create SSL connector: {}", e)))?;
            Ok(SslConnector::NativeTls(connector))
        }
        
        #[cfg(all(feature = "rustls", not(feature = "openssl-ssl"), not(feature = "native-tls")))]
        {
            let mut root_cert_store = rustls::RootCertStore::empty();
            
            let certs = rustls_native_certs::load_native_certs();
            for cert in certs.certs {
                root_cert_store
                    .add(cert)
                    .map_err(|e| PrivoxyError::Ssl(format!("Failed to add certificate: {}", e)))?;
            }
            
            let config = RustlsClientConfig::builder()
                .with_root_certificates(root_cert_store)
                .with_no_client_auth();
            Ok(SslConnector::Rustls(Arc::new(config)))
        }
        
        #[cfg(all(feature = "wolfssl", not(feature = "openssl-ssl"), not(feature = "native-tls"), not(feature = "rustls")))]
        {
            let connector = WolfsslSslConnector::new()
                .map_err(|e| PrivoxyError::Ssl(format!("Failed to create SSL connector: {}", e)))?;
            Ok(SslConnector::Wolfssl(connector))
        }
        
        #[cfg(not(any(feature = "openssl-ssl", feature = "native-tls", feature = "rustls", feature = "wolfssl")))]
        {
            Ok(SslConnector::None)
        }
    }
    
    pub async fn connect<S>(&self, stream: S, host: &str) -> PrivoxyResult<SslStream<S>>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        match self {
            #[cfg(feature = "openssl-ssl")]
            SslConnector::Openssl(connector) => {
                let ssl = connector
                    .configure()
                    .map_err(|e| PrivoxyError::Ssl(format!("Failed to configure SSL: {}", e)))?
                    .into_ssl(host)
                    .map_err(|e| PrivoxyError::Ssl(format!("Failed to create SSL: {}", e)))?;
                let ssl_stream = TokioOpensslSslStream::new(ssl, stream)
                    .map_err(|e| PrivoxyError::Ssl(format!("Failed to create SSL stream: {}", e)))?;
                Ok(SslStream::Openssl(ssl_stream))
            }
            
            #[cfg(feature = "native-tls")]
            SslConnector::NativeTls(connector) => {
                let connector = connector
                    .clone()
                    .builder()
                    .map_err(|e| PrivoxyError::Ssl(format!("Failed to configure SSL: {}", e)))?
                    .build();
                let ssl_stream = connector
                    .connect(host, stream)
                    .await
                    .map_err(|e| PrivoxyError::Ssl(format!("Failed to connect with SSL: {}", e)))?;
                Ok(SslStream::NativeTls(ssl_stream))
            }
            
            #[cfg(feature = "rustls")]
            SslConnector::Rustls(config) => {
                let server_name = ServerName::try_from(host.to_string())
                    .map_err(|e| PrivoxyError::Ssl(format!("Invalid server name: {}", e)))?;
                let connector = RustlsConnector::from(config.clone());
                let ssl_stream = connector
                    .connect(server_name, stream)
                    .await
                    .map_err(|e| PrivoxyError::Ssl(format!("Failed to connect with SSL: {}", e)))?;
                Ok(SslStream::Rustls(ssl_stream))
            }
            
            #[cfg(feature = "wolfssl")]
            SslConnector::Wolfssl(connector) => {
                let ssl_stream = connector
                    .connect(host, stream)
                    .await
                    .map_err(|e| PrivoxyError::Ssl(format!("Failed to connect with SSL: {}", e)))?;
                Ok(SslStream::Wolfssl(ssl_stream))
            }
            
            #[cfg(not(any(feature = "openssl-ssl", feature = "native-tls", feature = "rustls", feature = "wolfssl")))]
            SslConnector::None => {
                Ok(SslStream::None(stream))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssl_connector_new() {
        let connector = SslConnector::new();
        assert!(connector.is_ok());
    }
}

