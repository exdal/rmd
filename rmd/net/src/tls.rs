use std::{sync::Arc, time::Duration};

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::{
    DigitallySignedStruct,
    SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature},
    pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
};

use crate::{Error, fail};

pub(crate) const SERVER_NAME: &str = "rmd";

fn provider() -> Arc<CryptoProvider> { Arc::new(rustls::crypto::ring::default_provider()) }

fn transport() -> Arc<quinn::TransportConfig> {
    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(Duration::from_secs(5)));
    transport.max_idle_timeout(Some(quinn::IdleTimeout::from(quinn::VarInt::from_u32(20_000))));

    Arc::new(transport)
}

pub(crate) fn server_config() -> Result<quinn::ServerConfig, Error> {
    let certified = rcgen::generate_simple_self_signed(vec![SERVER_NAME.to_owned()]).map_err(fail)?;
    let key = PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());
    let crypto = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(fail)?
        .with_no_client_auth()
        .with_single_cert(vec![certified.cert.der().clone()], key.into())
        .map_err(fail)?;

    let mut config = quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(crypto).map_err(fail)?));
    config.transport_config(transport());

    Ok(config)
}

pub(crate) fn client_config() -> Result<quinn::ClientConfig, Error> {
    let provider = provider();
    let crypto = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(fail)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCert(provider)))
        .with_no_client_auth();

    let mut config = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(crypto).map_err(fail)?));
    config.transport_config(transport());

    Ok(config)
}

// host cert is trashed on every start
#[derive(Debug)]
struct AnyCert(Arc<CryptoProvider>);

impl ServerCertVerifier for AnyCert {
    fn verify_server_cert(
        &self, _end_entity: &CertificateDer<'_>, _intermediates: &[CertificateDer<'_>], _server_name: &ServerName<'_>,
        _ocsp_response: &[u8], _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
