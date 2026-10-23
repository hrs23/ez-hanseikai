# Running a relay server

Members who cannot reach each other directly talk through a relay server. The app uses iroh's public relays (Default) unless Settings > General > Relay server names one of your own. A relay sees only encrypted traffic, and only the relay in use is published with your ID.

1. On a host with a public name, install the relay of the same version as the app's `iroh` crate (1.3.0 here, see `src-tauri/Cargo.toml`):

   ```
   cargo install iroh-relay --version 1.3.0 --features server
   ```

2. Write `relay.toml`, with Let's Encrypt:

   ```toml
   enable_quic_addr_discovery = true

   [tls]
   cert_mode = "LetsEncrypt"
   hostname = "relay.example.org"
   contact = "admin@example.org"
   cert_dir = "certs"
   ```

   or with your own certificate: `cert_mode = "Manual"`, `manual_cert_path` and `manual_key_path` (PEM files).

3. Open TCP 80 and 443 and UDP 7842 (QUIC address discovery), and start it: `iroh-relay --config-path relay.toml`.
4. Check that `https://relay.example.org/` answers in a browser.
5. Put `https://relay.example.org/` in Settings > General > Relay server (Custom) on every member's PC.
