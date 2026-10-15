# QT

**Alpha version — not audited.** An E2EE messenger with post-quantum
cryptography — X3DH, PQXDH, Double Ratchet, Sealed Sender, Noise Protocol,
all implemented in Rust — with an Android app and a cloud relay, already
working end to end. **It is not a finished product yet and must not be
used for sensitive information** — see "Be honest with me: does this
actually work?" below.

[Ler isto em português →](README.md)

## Context

This project grew out of a genuine interest in privacy and systems
security — specifically, the question of how to build communication
where privacy is guaranteed by technical design and not only by a
written policy (the "data protection by design" that the GDPR asks for).
Rather than stopping at theory, the goal was to actually
implement and test every piece: not pseudocode, not diagrams — code that
compiles, runs, and has automated tests confirming it does what it claims.

## Why this exists (beyond the technical exercise)

Almost every popular messenger asks for your phone number to sign up.
Even when message content is private, that registration creates a
trail — who talks to whom, when, how often. For companies this is also
a compliance matter: the GDPR requires data protection by design and data
minimisation, and every piece of data a service keeps is data that can be
exposed in a breach.

QT tries to answer this with engineering, not just a privacy policy:
identity based on a cryptographic key (no phone, no email), end-to-end
encryption the server itself cannot break, and open source code so anyone
can verify there are no hidden tricks.

## Be honest with me: does this actually work?

Yes, end to end — as an **alpha** release. **It has not been independently audited and must not be used for real conversations or sensitive information.**

### Validated as of 15 October 2026

- Cryptographic core (PQXDH + Double Ratchet + Sealed Sender), transport, relay, local storage and FFI implemented in Rust, with the test suite passing (`cargo test --workspace`).
- Relay running on an AWS EC2 instance behind real TLS (`wss://`, Let's Encrypt certificate via Caddy), with Redis reachable only on `127.0.0.1` and protected by ACLs.
- Android app built as a signed *release* (V2/V3 schemes), native libraries aligned to 16 KB pages, no detections on VirusTotal (0/68).
- **Messages exchanged between two real phones through the relay on AWS.**
- **iPhone app (SwiftUI) on the same core, exchanging messages with the Android apps.**
- **Field test with five devices on three platforms** (2 real Android phones, 1 Android emulator, 1 iPhone and 1 Mac), with simultaneous, continuous sending: no messages lost. Details, problems found and what is still untested in [`docs/field-tests.md`](docs/field-tests.md) (in Portuguese).

### Still missing

- ~~Persistent history~~ — **done**: history stored in SQLCipher (in the same transaction that advances the ratchet), with *Sent / Delivered / Read* indicators via encrypted receipts and a persistent outbox.
- **Push notifications** — the app only receives messages while it is open.
- **Production infrastructure** — own domain (the current one is temporary), redundancy, backups and server alerts.
- App lock with PIN/biometrics, adding contacts via QR code, a more polished interface.
- Distribution through the Play Store (some antivirus apps currently warn because the app comes from an unknown source).
- Load testing and an **independent security audit**.

**Next stages:** *alpha* (now: works end to end, tested with a few people) → *beta* (persistent history, notifications, Play Store testing) → *1.0* (after an external audit).

## What's already built

| Piece | What it does | Confidence |
|---|---|---|
| `core/` | Session establishment (X3DH + post-quantum variant) and continuous encryption (Double Ratchet) — the same design used by Signal | ✅ Tested (14+ automated tests, including cases designed to fail) |
| `transport/` | Network connection protection (Noise Protocol) over WebSocket | ✅ Tested with real WebSocket connections, not simulated |
| `server/` | The "mailman": delivers messages without ever seeing the content or knowing who sent them | ✅ Tested, including surviving being force-killed (`kill -9`) |
| `storage/` | Stores your identity and conversations on disk, encrypted | ✅ Tested, including confirming a wrong password opens nothing |
| `cli/` | The messenger in the terminal (`messenger`), on the same core as the apps, plus a visible protocol demo (`messenger_demo`) | ✅ Tested end to end against a real relay: terminal ↔ terminal and terminal ↔ a client identical to the apps' |
| `ffi/` | The bridge connecting all of this to Kotlin (Android), Swift (iOS) and Python | ✅ Tested from Python |
| `android/` | Android app (Kotlin/Compose) — all cryptography stays in Rust | ✅ Signed release tested between two real phones through the relay on AWS |
| `ios/` | iPhone app (SwiftUI), same as the Android one, on the same Rust core | ✅ Built on a Mac and exchanging messages through the relay on AWS |
| `deploy/` | Relay install on EC2: Redis on localhost only with ACLs, sandboxed systemd service, automatic TLS | ✅ Used in the current deployment |

## What's missing, plainly

Beyond the list above ("Still missing"), longer term:

- Several independent servers collaborating (federation), so that one server failing does not stop the whole network
- An independent security audit, done by someone other than me

## Try it yourself

The fastest way to see this working is this command, which shows a full
encrypted conversation between two fictional people, step by step:

```bash
cargo run -p qt_cli --bin messenger_demo
```

### Requirements to build

```bash
sudo apt install redis-server libsqlcipher-dev
redis-server --daemonize yes
```
Rust via [rustup](https://rustup.rs) — any recent version works, except
the post-quantum variant of the core, which requires Rust 1.81 or newer.

### Running the tests

```bash
cargo test --workspace                          # everything except post-quantum
cargo test -p qt_core --features pq              # including post-quantum
```

### Two people talking through the relay (on your PC)

```bash
cargo run --release -p qt_server --bin relay_server &       # local relay on 127.0.0.1:9443

# in two separate terminals (each one is a "person"):
cargo run -p qt_server --example peer -- --relay ws://127.0.0.1:9443 --db alice.db
cargo run -p qt_server --example peer -- --relay ws://127.0.0.1:9443 --db bob.db
# in each: /id shows the Public ID; /add <the other's ID>; then type messages
```

The `peer` client uses exactly the same protocol as the Android app. Each person's identifier is derived from their public key — nobody chooses a "username," on purpose. `ws://` is only accepted for `localhost` in debug builds; otherwise the transport requires `wss://`.

### The messenger in the terminal (talks to the phones)

`messenger` uses the same core and the same relay as the apps: what you type on the PC reaches the phone, and the other way round.

```bash
cargo build --release -p qt_cli --bin messenger
export QT_RELAY_URL=wss://<your-relay>  QT_RELAY_PIN=<the relay's Noise public key>
M=./target/release/messenger

$M identity --db me.sqlite                 # once: creates the identity and prints your QT Public ID
$M add --db me.sqlite Ana qt1:...          # saves a contact (the ID their app shows)
$M send --db me.sqlite --to Ana            # the message is asked for in the terminal
$M listen --db me.sqlite                   # keeps receiving (Ctrl+C to quit)
$M history --db me.sqlite Ana              # history, stored encrypted
$M remove --db me.sqlite Ana               # deletes the contact, its messages and the conversation keys
```

The passphrase is always asked for in the terminal, hidden (or read from one line of stdin with `--passphrase-stdin`, for scripts). Argon2id turns it into the database key and the key for the contact list, which is encrypted too.

### Generating Android/iOS/Python bindings

```bash
./ffi/generate_bindings.sh
```

## Android app

```bash
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/<version>
ABIS="arm64-v8a armeabi-v7a" ./scripts/build-android.sh   # libqt_ffi.so (16 KB) + Kotlin bindings
cd android && ./sync-native.sh ..                          # copy into the app
# then open the android/ folder in Android Studio
```

The *release* build requires your own keystore and the production relay (`qtRelayUrl` / `qtRelayPin` in `android/gradle.properties`); in that build the relay is fixed and not shown in the UI. Step by step in [`android/README.md`](android/README.md) (Portuguese).

## Relay on AWS

On a fresh EC2 instance with Ubuntu 24.04 (Security Group with only 22, 80 and 443):

```bash
curl -fsSLO https://raw.githubusercontent.com/moral666/QT/security-hardening/deploy/aws/setup-ec2.sh
sudo bash setup-ec2.sh
```

It installs Redis (`127.0.0.1` only, minimal-ACL user), the relay as a sandboxed systemd service and Caddy with automatic TLS, and prints the `wss://` URL and pinning key for the app. Details in [`docs/deploy-wss.md`](docs/deploy-wss.md).

## How the project is organized

```
core/        The heart: end-to-end encryption, no external dependencies
transport/   Protects the network connection
server/      Delivers messages without being able to read them
storage/     Stores everything on disk, encrypted
cli/         The messenger in the terminal (same core and relay as the apps)
demo/        Combines core+transport into a single test, to see it all in action
ffi/         The bridge to Kotlin, Swift and Python
android/     The Android app (Kotlin/Compose)
deploy/      Relay server install (AWS, Redis, systemd)
scripts/     Building the native libraries for Android
docs/        The full technical specification, and the design history
```

Each piece has its own tests, kept next to the code (`tests/`).

## Want the technical details?

- [`docs/protocol-spec.md`](docs/protocol-spec.md) explains, section by section, exactly how each piece works internally — which algorithms, which decisions, and what's still missing in each one.
- [`docs/threat-model.md`](docs/threat-model.md) explains who and what this protects against — and, just as importantly, who and what it does **not** protect against. Recommended reading before trusting any security claim made here.
- [`ROADMAP.md`](ROADMAP.md) shows what's left, in priority order.
- [`CHANGELOG.md`](CHANGELOG.md) shows the history of what's already changed.

## License

AGPL-3.0-or-later. In plain terms: anyone can use, study and modify this
code freely — but if someone takes it to offer a service (even just over
the internet, without "distributing" software in the traditional sense),
they're required to also share the source of those modifications. This
closes a door that more permissive licenses leave open: someone taking an
open project and turning it into a closed service.

## Want to help or report a problem?

- `CONTRIBUTING.md` explains how to propose changes, and has extra
  (reasonable) rules for anyone touching cryptography code
- `SECURITY.md` explains how to report a vulnerability privately — please
  don't open a public issue for that
