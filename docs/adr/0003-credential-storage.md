# ADR 0003: Camera credential storage and DPAPI scope

- Status: accepted
- Date: 2026-11-19
- Phase: 4

## Context

Cameras need credentials (SPEC §4.5). SPEC §4.6 requires them to be encrypted
before reaching SQLite, write-only through the API, never present in
`Debug`/logs, reusable as named credential sets, and protected on Windows with
DPAPI. The default scope is machine scope with app-specific entropy, so the app
keeps working if it later runs as a Windows service (§18 Q2).

## Decision

### Store
A `CredentialStore` trait with `encrypt`/`decrypt` (plus `seal`/`open` for
`Credentials`). Each ciphertext starts with a backend tag, so moving a database
to a machine with another backend fails clearly instead of returning garbage.

- **Windows: `dpapi1:`.** `CryptProtectData` with `CRYPTPROTECT_LOCAL_MACHINE |
  CRYPTPROTECT_UI_FORBIDDEN` and the entropy `"CCANH camera credentials v1"`.
  Called through `windows-sys` (no extra runtime dependency). Tested on the
  Windows CI runner.
- **Elsewhere: `aesgcm1:`.** AES-256-GCM (RustCrypto `aes-gcm`), with a random
  96-bit nonce per message, the same string as associated data, and a 32-byte
  random key in `<data_dir>/keys/credentials.key` (mode 0600 on Unix). A
  startup warning says it's a development store.

The plaintext is a small JSON object `{username, password}`: both are
encrypted, so usernames don't leak either. Decrypted buffers are moved into
`secrecy::SecretString` so they are zeroed on drop.

### Data model
One `credential_sets` table, holding ciphertext only:
- **Shared sets:** `shared = 1`, unique case-insensitive name, reusable by many
  cameras. They can't be deleted while in use.
- **Private sets:** `shared = 0`, no name. These are credentials entered for a
  single camera. They're deleted when the camera is deleted or switched to other
  credentials.

`cameras.credential_set_id` references the set (`ON DELETE RESTRICT`). The API
accepts `{"type": "none" | "set" | "inline"}` and returns only
`has_credentials` plus the shared set's `{id, name}`.

### Use
Credentials are decrypted only when a provider is built: by the status monitor
(rebuilt when the camera or its shared set changes), refresh, and test
connection. They live in memory inside the provider as `SecretString`. The
HTTP client marks `Authorization` headers as sensitive and never puts
credentials in URLs or error text. A TRACE-level log test covers credential
sets, inline credentials and Digest authentication.

## Consequences

- **Machine scope means any administrator (or process running as SYSTEM) on
  the server can decrypt the credentials.** This is the accepted trade-off for
  service-mode compatibility. Protect the host and restrict the data
  directory's ACL.
- A database copied to another machine can't decrypt credentials. Admins
  re-enter them, or keep a backup of the machine's DPAPI keys per Windows
  guidance.
- The AES-GCM key file is as sensitive as the credentials. It's acceptable for
  development and CI only.
- Switching to user scope later would mean re-encrypting all sets. The backend
  tag (`dpapi1`) would be bumped to tell the formats apart.
