# QT

<p align="center"><img src="branding/sealer-logo-transparente.png" width="140" alt="SEALER logo"></p>

The apps are called **SEALER**; QT is the name of the protocol and of this repository.

**Alpha version, not audited.** An E2EE messenger with post-quantum
cryptography (X3DH, PQXDH, Double Ratchet, Sealed Sender, Noise Protocol),
all implemented in Rust, with Android and iPhone apps and a cloud relay,
already working end to end. **It is not a finished product yet and must not be
used for sensitive information.** See "Be honest with me: does this
actually work?" below.

[Ler isto em português →](README.md)

## Context

This project grew out of a genuine interest in privacy and systems
security, specifically the question of how to build communication
where privacy is guaranteed by technical design and not only by a
written policy (the "data protection by design" that the GDPR asks for).
Rather than stopping at theory, the goal was to actually
implement and test every piece: not pseudocode, not diagrams, but code that
compiles, runs, and has automated tests confirming it does what it claims.

## Why this exists (beyond the technical exercise)

Almost every popular messenger asks for your phone number to sign up.
Even when message content is private, that registration creates a
trail: who talks to whom, when, how often. For companies this is also
a compliance matter: the GDPR requires data protection by design and data
minimisation, and every piece of data a service keeps is data that can be
exposed in a breach.

QT tries to answer this with engineering, not just a privacy policy:
identity based on a cryptographic key (no phone, no email), end-to-end
encryption the server itself cannot break, and open source code so anyone
can verify there are no hidden tricks.

## Be honest with me: does this actually work?

Yes, end to end, as an **alpha** release. **It has not been independently audited and must not be used for real conversations or sensitive information.**

### Validated as of 15 October 2026

- Cryptographic core (PQXDH + Double Ratchet + Sealed Sender), transport, relay, local storage and FFI implemented in Rust, with the test suite passing (`cargo test --workspace`).
- Relay running on an AWS EC2 instance behind real TLS (`wss://`, Let's Encrypt certificate via certbot and HAProxy), with Redis reachable only on `127.0.0.1` and protected by ACLs.
- Android app built as a signed *release* (V2/V3 schemes), native libraries aligned to 16 KB pages, no detections on VirusTotal (0/68).
- **Messages exchanged between two real phones through the relay on AWS.**
- **iPhone app (SwiftUI) on the same core, exchanging messages with the Android apps.**
- **Field test with five devices on three platforms** (2 real Android phones, 1 Android emulator, 1 iPhone and 1 Mac), with simultaneous, continuous sending: no messages lost. Details, problems found and what is still untested in [`docs/field-tests.md`](docs/field-tests.md) (in Portuguese).
- **Relay with 10,000 simultaneous connections** (each one polling for messages every 3 seconds, like the apps): no failures, about **14 KB of memory per connection**. See "The relay in numbers" below.

### New on 16 and 17 October 2026

- **Instant delivery, tested with real phones:** the typical time for a message to reach the other phone went from **about 2,000 ms to 300 to 430 ms**. See "Real-world times, between phones" below.
- **Deleting the account also on the server:** "Destroy profile and data" deletes the public keys and waiting messages on the relay, then everything on the phone.
- **Lighter on the battery (Android):** 30 seconds after leaving the app, it closes its connections to the relay and reconnects when you open it again.
- **Getting ready for Google Play:** privacy policy and account deletion page on sealer.pt, and answers for the Play Console forms in [`docs/google-play.md`](docs/google-play.md) (in Portuguese).

### Just added, still to be confirmed in real use

- **Images and GIFs up to 25 MB**, in both apps. Each file is encrypted on the phone with its own key, goes to the relay in chunks and is deleted there once the other phone downloads it, or after 24 hours. Covered by the automated tests (including against the production Redis command list); the exchange between phones through AWS is still being confirmed. Details in [`docs/protocol-spec.md`](docs/protocol-spec.md), section 14 (in Portuguese).
- **Adding contacts with a QR code**, in both apps (the code is generated and read on the phone, with no network involved).
- SEALER branding in the apps (logo, name and theme).

The iPhone app with these additions has not been built in Xcode yet (see [`ios/README.md`](ios/README.md), in Portuguese).

### Still missing

- ~~Persistent history~~ (**done**): history stored in SQLCipher (in the same transaction that advances the ratchet), with *Sent / Delivered / Read* indicators via encrypted receipts and a persistent outbox.
- ~~Adding contacts via QR code~~ (done in both apps).
- **Push notifications**: the app only receives messages while it is open.
- **Production infrastructure**: own domain (the current one is temporary), redundancy, backups and server alerts.
- ~~App lock~~ (done on Android): an app passcode that encrypts the key to the conversations (Argon2id), plus optional unlock with fingerprint, face or the phone's PIN. Still missing on iPhone.
- Distribution through the Play Store (some antivirus apps currently warn because the app comes from an unknown source). Already prepared: account deletion, privacy policy and form answers ([`docs/google-play.md`](docs/google-play.md), in Portuguese).
- Load testing on the AWS relay (already measured on a local machine, see [`docs/performance.md`](docs/performance.md), in Portuguese) and an **independent security audit**.

**Next stages:** *alpha* (now: works end to end, tested with a few people) → *beta* (notifications, app lock on iPhone, Play Store testing) → *1.0* (after an external audit).

## Architecture

<!-- Generated by site/scripts/architecture_diagram.py (see site/README.md, section 14). -->
![SEALER architecture: the Android and iPhone apps use the same QT core in Rust and exchange encrypted envelopes through a relay on AWS (Caddy, a Rust relay and Redis). The sealer.pt website is separate, on Cloudflare.](docs/architecture.svg)

## What's already built

| Piece | What it does | Confidence |
|---|---|---|
| `core/` | Session establishment (X3DH + post-quantum variant) and continuous encryption (Double Ratchet), the same design used by Signal | ✅ Tested (over 40 automated tests, including cases designed to fail) |
| `transport/` | Network connection protection (Noise Protocol) over WebSocket | ✅ Tested with real WebSocket connections, not simulated |
| `server/` | The "mailman": delivers messages without ever seeing the content or knowing who sent them, and keeps encrypted images for 24 hours at most | ✅ Tested, including surviving being force-killed (`kill -9`) and against the production Redis command list |
| `storage/` | Stores your identity and conversations on disk, encrypted | ✅ Tested, including confirming a wrong password opens nothing |
| `cli/` | The messenger in the terminal (`messenger`), on the same core as the apps, plus a visible protocol demo (`messenger_demo`) | ✅ Tested end to end against a real relay: terminal ↔ terminal and terminal ↔ a client identical to the apps' |
| `ffi/` | The bridge connecting all of this to Kotlin (Android), Swift (iOS) and Python: history, outbox, images, app lock | ✅ Tested in Rust (including crossed sessions and images) and from Python |
| `android/` | Android app (Kotlin/Compose); all cryptography stays in Rust | ✅ Signed release tested between two real phones through the relay on AWS |
| `ios/` | iPhone app (SwiftUI), same as the Android one, on the same Rust core | ✅ Built on a Mac and exchanging messages through the relay on AWS (the latest additions not built yet) |
| `deploy/` | Relay install on EC2: Redis on localhost only with ACLs, sandboxed systemd service, automatic TLS | ✅ Used in the current deployment |

## The relay in numbers

The relay is small on purpose: it only passes encrypted messages along, keeps no history (encrypted images stay there for 24 hours at most), and the heavy cryptography runs on the phones. Measured with 10,000 connections open at the same time, each one polling for messages every 3 seconds, as the apps do while open:

| | Result |
|---|---|
| Connections open at the same time | **10,000 of 10,000, 0 failures** |
| Relay memory | **138 MB in total, about 14 KB per connection** (starts at 4 MB) |
| Requests answered | **~3,300 per second, 0 errors** |
| CPU per request | about 0.05 ms in the relay and 0.02 ms in Redis (24 % of one core for all 10,000 connections) |
| CPU with idle connections | 0 % |

With messages flowing all the time (50 pairs of connections sending as fast as possible), the relay delivered about **16,900 messages of 200 B per second** and **6,000 of 4 KB**, with 95 % of messages reaching the recipient in under 10 ms (200 B) and 26 ms (4 KB). A 25 MB image was uploaded to the relay in 0.3 s and downloaded in 0.4 s.

Measured on a local 2 vCPU machine, without the proxy's TLS and without a real network in between, so the full server uses more than this. How it was measured, how to repeat it and what was left out: [`docs/performance.md`](docs/performance.md) (in Portuguese).

## Real-world times, between phones

The numbers above are for the relay on its own. With real phones (three Samsungs, in Viseu and Cascais, on Wi-Fi and mobile data, with the relay in Stockholm), measured by the relay itself in October 2026 over hundreds of messages:

| | Before (16 Oct) | Now (17 Oct) |
|---|---|---|
| From the server until the other phone has the message (typical time, median) | **about 2,000 ms** | **300 to 430 ms** |
| From the sender's phone to the server (average) | | **about 50 ms** |

The big difference came from **instant delivery**: before, the apps asked the relay every 3 seconds whether there were new messages; now they wait on an open connection and the relay delivers each message the moment it arrives. What is left is mostly the network round trip to Sweden and back, plus the phone decrypting and storing the message. With the relay closer (for example, in Spain), these times should drop further.

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
Rust 1.81 or newer, via [rustup](https://rustup.rs) (the post-quantum part requires it).

### Running the tests

```bash
cargo test --workspace                          # everything, post-quantum included (needs Redis, above)
cargo test -p qt_core --features pq              # the cryptographic core only
```

The `server/tests/redis_acl.rs` test starts its own Redis with the production command list; without `redis-server` installed it is skipped.

### Two people talking through the relay (on your PC)

```bash
cargo run --release -p qt_server --bin relay_server &       # local relay on 127.0.0.1:9443

# in two separate terminals (each one is a "person"):
cargo run -p qt_server --example peer -- --relay ws://127.0.0.1:9443 --db alice.db
cargo run -p qt_server --example peer -- --relay ws://127.0.0.1:9443 --db bob.db
# in each: /id shows the Public ID; /add <the other's ID>; then type messages
```

The `peer` client uses exactly the same protocol as the Android app. Each person's identifier is derived from their public key: nobody chooses a "username," on purpose. `ws://` is only accepted for `localhost` in debug builds; otherwise the transport requires `wss://`.

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

The Rust core is the only cryptographic backend of the Android app (Kotlin/Compose): `QtClient` API via UniFFI, SQLCipher storage with a key from the Android Keystore, PQXDH (X25519 + ML-KEM-768) + Double Ratchet + Sealed Sender, authenticated relay with *at-least-once* delivery. **Alpha version, not audited.**

```bash
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/<version>
ABIS="arm64-v8a armeabi-v7a" ./scripts/build-android.sh   # libqt_ffi.so (16 KB) + Kotlin bindings
cd android && ./sync-native.sh ..                          # copy into the app
# then open the android/ folder in Android Studio
```

The *release* build requires your own keystore and the production relay (`qtRelayUrl` / `qtRelayPin` in `android/gradle.properties`); in that build the relay is fixed and not shown in the UI. Use NDK 27 or newer (with r26d the build of the bundled OpenSSL failed). Step by step in [`android/README.md`](android/README.md) (Portuguese).

* Integration and build guide: [`docs/android-integration.md`](docs/android-integration.md)
* API reference: [`docs/ffi-api.md`](docs/ffi-api.md)
* Protocol (§8–14 = current formats): [`docs/protocol-spec.md`](docs/protocol-spec.md)
* Threats and limitations: [`docs/threat-model.md`](docs/threat-model.md), [`docs/known-limitations.md`](docs/known-limitations.md)

(These documents are in Portuguese.)

## iPhone app

Needs a Mac with Xcode and [XcodeGen](https://github.com/yonaskolb/XcodeGen):

```bash
./scripts/build-ios.sh          # builds the Rust core for iPhone and the simulator
cd ios && xcodegen && open SealerMessenger.xcodeproj
```

Step by step, Apple accounts and the differences from Android in [`ios/README.md`](ios/README.md) (Portuguese).

## Relay on AWS

On a fresh EC2 instance with Ubuntu 24.04 (Security Group with only 22, 80 and 443):

```bash
curl -fsSLO https://raw.githubusercontent.com/moral666/QT/security-hardening/deploy/aws/setup-ec2.sh
sudo bash setup-ec2.sh
```

It installs Redis (`127.0.0.1` only, minimal-ACL user), the relay as a sandboxed systemd service and HAProxy with a TLS certificate (certbot, automatic renewal), and prints the `wss://` URL and pinning key for the app. Encrypted images are kept in `/var/lib/qt/anexos` (2 GB in total and 24 hours each at most, by default).

To update a relay that is already installed, run the same script again: it fetches the new code and rebuilds, but keeps the Noise key (the apps' pin does not change) and the Redis password. The build runs on the instance itself and needs room: an 8 GB disk ended up almost full, the current relay uses 20 GB. Details in [`docs/deploy-wss.md`](docs/deploy-wss.md).

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
ios/         The iPhone app (SwiftUI)
branding/    Logo, icons and font of the SEALER brand
site/        The sealer.pt website (static page on Cloudflare)
deploy/      Relay server install (AWS, Redis, systemd)
scripts/     Builds for Android and iPhone, and measurements (relay and phone)
docs/        The full technical specification, and the design history
```

Each piece has its own tests, kept next to the code (`tests/`).

## Want the technical details?

- [`docs/protocol-spec.md`](docs/protocol-spec.md) explains, section by section, exactly how each piece works internally: which algorithms, which decisions, and what's still missing in each one.
- [`docs/threat-model.md`](docs/threat-model.md) explains who and what this protects against and, just as importantly, who and what it does **not** protect against. Recommended reading before trusting any security claim made here.
- [`docs/known-limitations.md`](docs/known-limitations.md) lists the known limitations, by impact.
- [`docs/field-tests.md`](docs/field-tests.md) and [`docs/performance.md`](docs/performance.md) record the tests with real devices and the relay measurements.
- [`ROADMAP.md`](ROADMAP.md) shows what's left, in priority order.
- [`CHANGELOG.md`](CHANGELOG.md) shows the history of what's already changed.

(The documents in `docs/`, the roadmap and the changelog are in Portuguese.)

## License

QT is offered under a dual license:

- **Open source:** GNU Affero General Public License, version 3 or later (AGPL-3.0-or-later). In plain terms: anyone can use, study and modify this code freely, but if someone takes it to offer a service (even just over the internet, without "distributing" software in the traditional sense), they're required to also share the source of those modifications.
- **Commercial:** organisations that need terms other than the AGPL can obtain a commercial license through a written agreement with the author.

The commercial license allows specific terms of use, integration, distribution, deployment, support, maintenance and other commercial rights to be set by contract.

Third-party components and dependencies used by QT remain subject to their own licenses.

See [`LICENSE`](LICENSE) for the AGPL and [`LICENSE-COMMERCIAL.md`](LICENSE-COMMERCIAL.md) for the commercial licensing model.

## Want to help or report a problem?

- `CONTRIBUTING.md` explains how to propose changes, and has extra
  (reasonable) rules for anyone touching cryptography code
- `SECURITY.md` explains how to report a vulnerability privately. Please
  don't open a public issue for that
