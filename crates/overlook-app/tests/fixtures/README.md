# Synthetic TLS fixtures

`resource-test-cert.der` and `resource-test-key.der` are a public, disposable test
certificate and key for `synthetic.example` and `cdn.synthetic.example`. They are
used only inside `#[cfg(test)]` by a loopback fake resource server. Never deploy
this key or add this certificate to system trust. The production client continues
to use public CA roots and public-IP DNS filtering.

Generated on 2026-10-22 with OpenSSL, RSA-2048 / SHA-256, SANs above, CA:FALSE,
10-year validity. These tests exercise actual TLS with certificate verification;
they do not disable verification or accept plaintext HTTP in production.
