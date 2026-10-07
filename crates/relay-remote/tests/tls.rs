//! The `wss://` route needs rustls to have a crypto provider. tokio-tungstenite builds its client
//! config with `ClientConfig::builder()`, which panics without one — the tunnel task died on its
//! first TLS dial. Its own process and one test, so nothing installed a provider beforehand.

#[test]
fn rustls_has_a_provider_from_crate_features_and_from_the_tunnel() {
    assert!(rustls::crypto::CryptoProvider::get_default().is_none());
    let _ = rustls::ClientConfig::builder().with_root_certificates(rustls::RootCertStore::empty()).with_no_client_auth();

    relay_remote::tunnel::ensure_crypto_provider();
    assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    relay_remote::tunnel::ensure_crypto_provider();
}
