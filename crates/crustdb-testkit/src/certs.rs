use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};

/// CA plus a server certificate (SAN = requested hostnames), also written to `dir`
/// as `ca.pem`, `server.crt`, `server.key`. Also a client certificate signed by the
/// same CA (`client.crt`, `client.key`).
pub struct TestCerts {
    pub ca_pem: String,
    pub server_cert_pem: String,
    pub server_key_pem: String,
    pub client_cert_pem: String,
    pub client_key_pem: String,
    pub dir: tempfile::TempDir,
}

pub fn certs_for(hostnames: &[&str]) -> TestCerts {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "crustdb test CA");
    let ca = ca_params.self_signed(&ca_key).unwrap();

    let server_key = KeyPair::generate().unwrap();
    let mut params =
        CertificateParams::new(hostnames.iter().map(|h| h.to_string()).collect::<Vec<_>>())
            .unwrap();
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        hostnames.first().copied().unwrap_or("localhost"),
    );
    let issuer = rcgen::Issuer::new(ca_params, ca_key);
    let server = params.signed_by(&server_key, &issuer).unwrap();

    let client_key = KeyPair::generate().unwrap();
    let mut client_params = CertificateParams::new(Vec::new()).unwrap();
    // CN matches the PostgreSQL role used by the mTLS tests.
    client_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "postgres");
    let client = client_params.signed_by(&client_key, &issuer).unwrap();

    let t = TestCerts {
        ca_pem: ca.pem(),
        server_cert_pem: server.pem(),
        server_key_pem: server_key.serialize_pem(),
        client_cert_pem: client.pem(),
        client_key_pem: client_key.serialize_pem(),
        dir: tempfile::tempdir().unwrap(),
    };
    for (name, body) in [
        ("ca.pem", &t.ca_pem),
        ("server.crt", &t.server_cert_pem),
        ("server.key", &t.server_key_pem),
        ("client.crt", &t.client_cert_pem),
        ("client.key", &t.client_key_pem),
    ] {
        std::fs::write(t.dir.path().join(name), body).unwrap();
    }
    t
}
