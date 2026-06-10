# Security

Report security issues privately through the project maintainers.

The Phase 1 HTTP API binds to `127.0.0.1` only and is intended for local
dogfooding. GitHub tokens are held in memory only and must never be logged or
persisted.

TODO(security): loopback-only binding does not fully protect mutating endpoints
such as `POST /projection/approve` from local processes or malicious local web
contexts. Phase 1 intentionally defers per-run bearer-token/CSRF work because
this HTTP surface is temporary and test-heavy.

TODO(security): evaluate OS keychain token storage for Phase 2 desktop sessions.
