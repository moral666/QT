# QT

<p align="center"><img src="branding/sealer-logo-transparente.png" width="140" alt="SEALER logo"></p>

**SEALER** is a private messenger: no phone number, no email, end-to-end encryption and post-quantum protection. **QT** is the name of the protocol and of this repository.

**Status:** test version, in closed testing. Android and iPhone apps **0.9.51**, with two federated relays in the European Union (Stockholm and Spain).

[Ler em português →](README.md)

<p align="center">
  <img src="site/public/ecras/conversas-0.9.11.jpg" width="180" alt="Home screen: purple header with the logo, the profile photo and the ID, and the chat list">
  <img src="site/public/ecras/chat-0.9.11.jpg" width="180" alt="An end-to-end encrypted chat">
  <img src="site/public/ecras/definicoes-0.9.11.jpg" width="180" alt="Settings screen">
  <img src="site/public/ecras/novo-contacto-0.9.11.jpg" width="180" alt="New contact screen with the QR code">
</p>
<p align="center"><sub>Android app (screens in Portuguese). Names and messages are examples.</sub></p>

## Why it exists

Almost every messenger asks for your phone number. Even with encrypted messages, that record leaves a trail: who talks to whom, when and how often. QT answers this with engineering, not just a privacy policy:

- **Your identity is a cryptographic key**, created on your phone. No phone number, email or username.
- **End-to-end encryption** that not even the server can open. The server also does not know who sent each message.
- **Open source**, so anyone can check what the app does.

It is also a working example of "data protection by design", as the GDPR asks: every piece was built and tested for real, with automated tests confirming it does what it says.

## What the app does

- **One-to-one and group chats** (up to 10 people, 3 admins), between Android and iPhone, with *Sent / Delivered / Read*.
- **Encrypted voice and video calls** on Android and iPhone, also between the two. On Android they ring with the app closed or the phone locked, with their own ringtone.
- **Photos, GIFs, voice messages and files up to 25 MB.** Each file has its own key and leaves the server once downloaded (or after 24 hours).
- **Notifications with the app closed**, always generic ("You have new messages"): no name, no text.
- **Profile photo** only for your contacts, encrypted (the server does not even know it is a photo). It can be changed every 72 hours.
- **QR code invites**, reactions, blocking contacts and "online" shown only to contacts.
- **Badges** (Founder, Co-founder, Dev, Tester, Contributor, Bug Hunter, Support, Troll and Legacy): a ring and a badge on the photo (for Legacy, a shield around the photo), with a signed certificate, sent encrypted only to contacts.
- **Report a contact:** the reason and up to 20 messages go encrypted to the SEALER team; the server stores the report without reading it or knowing who made it, and the contact is blocked.
- **App lock** with its own code, fingerprint or face (Face ID on iPhone). After 5 wrong codes the wait goes up to 15 minutes, and changing the phone's clock does not skip it.
- **Recovery key:** 12 words bring back the ID, contacts, block list and badge on a new phone. The copy is stored encrypted on the server; history and groups are not in it.
- **Ten languages:** Portuguese, English, French, Spanish, German, Italian, Japanese, Simplified Chinese, Hindi and Arabic (right to left).
- **Two servers, one network:** your account lives on one of the federated relays, and you talk to people on the other without noticing (see [Federation](#federation-two-relays-one-network)).
- **Delete your account** from the phone and the server with one tap.

## How it protects privacy

- **Signal-style protocol:** PQXDH (X25519 + ML-KEM-768, resistant to quantum computers) to start a conversation, Double Ratchet for each message and Sealed Sender to hide the sender.
- **Rotating keys:** the keys published on the server (signed pre-key and ML-KEM) rotate every 7 days, and there are one-time keys, one per new conversation.
- **Same-size messages:** text is padded to 4 fixed sizes, so the server cannot tell a short message from a receipt.
- **Protected connection:** Noise Protocol on top of TLS (`wss://`), with the server key pinned in the apps.
- **Everything encrypted on the phone:** history in SQLCipher, with the key kept in the Android Keystore or the iPhone Secure Enclave (this device only, outside cloud backups). The recent-apps thumbnail does not show your chats.
- **Direct calls only between mutual contacts:** when both have each other in their contacts, the call connects phone to phone after it is answered, and in that case each one sees the other's IP. Otherwise (or if one blocks or deletes the other), it goes through the call server and the IP is not shared. Always end-to-end encrypted; through the server, audio packets all have the same size and rhythm, and the call server does not know which accounts are talking.
- **Video never goes through the server:** the picture in video calls only travels over the direct connection between mutual contacts. Without a direct connection, the call carries on with voice only.
- **Server defences:** proof of work to create an account, limits per account and per IP, and only registered accounts can send.

What this does and does not protect against is in [`docs/threat-model.md`](docs/threat-model.md) and [`docs/known-limitations.md`](docs/known-limitations.md) (in Portuguese).

## Federation: two relays, one network

Since 29 Oct 2026 SEALER has **two federated relays**, both in the European Union: `estocolmo` (AWS eu-north-1) and `espanha` (AWS eu-south-2). Each account lives on one relay only, and people on one talk to people on the other as if it were the same server.

- **The ID does not change:** it is still the public key (`qt1:...`). Relays ask each other where each account lives, and an account can only exist on one relay.
- **The app only talks to its own relay**, over one connection. Messages, keys to start conversations, attachments and the recovery copy travel from relay to relay.
- **The sender stays sealed:** the destination relay only learns that the message came from the other relay, not from whom.
- **Closed federation:** only relays on an approved list talk to each other, over `wss://<relay>/fed`, with Noise_XX and the keys pinned on both sides. HAProxy only lets through the IPs of the relays on the list.
- **Calls across relays:** both apps use the call relay of the caller.
- **No request logs:** the information "this account is on relay X" is kept in memory only (up to 24 hours), and relays only count how many messages they exchanged.

The design accepts more relays. The relay-to-relay protocol is in section 25 of [`docs/protocol-spec.md`](docs/protocol-spec.md) and the limits in [`docs/known-limitations.md`](docs/known-limitations.md) (14, in Portuguese).

## Status and next steps

**Tested:**

- The whole Rust core (cryptography, transport, server, storage and the bridge to the apps), with the test suite passing.
- Two federated relays on AWS: `wss://relay.sealerapp.com` (Stockholm) and `wss://relay-es.sealerapp.com` (Spain), each with Redis only on `127.0.0.1` and protected by an ACL.
- Signed *release* Android app on three Samsung phones and the emulator.
- **Android 0.9.16 with iPhone 0.9.15** (20/10/2026), no bugs: adding, deleting and re-adding contacts, QR code, share ID button, all menus, text and voice messages, GIFs, sending files, blocking contacts, deleting chats and voice calls between Androids, between Android and iPhone and between iPhones.
- **Android 0.9.22** (22/10/2026): app lock, recent apps and account recovery.
- **Android 0.9.31** (23/10/2026), no bugs: testers' requests (ringback for the caller, speaker before the call is answered, minimized call, copying the 12 words, links, badges after account recovery) and all ten languages reviewed, Arabic included.
- Field test with five devices on three platforms and continuous sending: no message lost ([`docs/field-tests.md`](docs/field-tests.md)).

**Next:**


- On Android, testing on real phones the integration with the system call service (Bluetooth, car, a regular call at the same time), done in 0.9.52.
- Testing on real phones the Community (people with a badge, shown only with their photo and badge), done in 0.9.52 in both apps.
- Distribution through Google Play and the App Store (today testers install the app by hand). Already prepared: [`docs/google-play.md`](docs/google-play.md).
- Production server with backups and alerts.

**Stages:** *closed testing* (now, 0.9.x) → *beta* in the stores, with more testers → *1.0*. The full plan is in [`ROADMAP.md`](ROADMAP.md).

## Architecture

<!-- Generated by site/scripts/architecture_diagram.py (see site/README.md, section 14). -->
![SEALER architecture: the Android and iPhone apps use the same QT core in Rust and exchange encrypted envelopes through two federated relays on AWS, in Stockholm and in Spain (HAProxy, Rust relay and Redis on each). The sealerapp.com website is separate, on Cloudflare.](docs/architecture.svg)

Both apps use the **same Rust core** through UniFFI. All cryptography lives in Rust; Kotlin and Swift only draw the screens.

| Piece | What it does |
|---|---|
| `core/` | Starting a session (X3DH and PQXDH) and encrypting each message (Double Ratchet, Sealed Sender) |
| `transport/` | Connection protection (Noise Protocol over WebSocket) |
| `server/` | The "postman": delivers messages without reading them or knowing who sent them, also from relay to relay (federation). Includes the call relay (UDP, anonymous rooms) |
| `call/` | Call engine: Opus 1.6 with DRED and neural PLC, adaptive jitter buffer |
| `storage/` | Identity and chats on disk, encrypted (SQLCipher) |
| `ffi/` | The bridge to Kotlin, Swift and Python: history, outbox, attachments, groups, calls |
| `android/` | Android app (Kotlin + Compose) |
| `ios/` | iPhone app (SwiftUI) |
| `cli/` | Terminal messenger, with the same core and server as the apps |
| `deploy/` | Server installation on EC2 (Redis with ACL, isolated systemd, automatic TLS) |

## Numbers

**Server alone** (2 vCPU machine, no TLS and no real network): 10,000 open connections without failures, about 14 KB per connection, ~3,300 requests per second and 0 % CPU with idle connections. With messages flowing constantly, ~16,900 messages of 200 B per second. With HAProxy and TLS: 8,000 connections without failures, about 40 KB per connection. Since 0.9.45 each app opens a single connection to the relay (it used to be three). Details in [`docs/performance.md`](docs/performance.md).

**Between real phones** (three Samsung phones in Portugal, server in Stockholm, 19 Oct 2026):

| | Typical message (median) | 95 % of messages |
|---|---|---|
| From the phone to the server | **42 ms** | 57 ms |
| From the server to the other phone | **141 ms** | 18 s |

The slow messages are for sleeping phones: the notification wakes them up and the message arrives when the app opens.

**Calls:** about 64 ms of network one way between Portugal and Stockholm, and about 35 MB per hour in each direction. In a test with 15 % of packets lost, DRED rebuilt them all. Switching from Wi-Fi to mobile data does not drop the call, and there is a fallback through `wss://relay.sealerapp.com/call` for networks that block UDP. Formats in section 18 of [`docs/protocol-spec.md`](docs/protocol-spec.md).

## For developers

### Try it

```bash
cargo run -p qt_cli --bin messenger_demo     # a full encrypted conversation, step by step
```

### Build and test

```bash
sudo apt install redis-server libsqlcipher-dev cmake
redis-server --daemonize yes
cargo test --workspace                       # everything (needs Redis)
cargo test -p qt_core --features pq          # only the cryptographic core
```

Rust 1.88 or newer, via [rustup](https://rustup.rs). `cmake` builds Opus for the calls. The `server/tests/redis_acl.rs` test starts a Redis with the production command list and is skipped without `redis-server`.

### Two people through the server, on your PC

```bash
cargo run --release -p qt_server --bin relay_server &       # local server on 127.0.0.1:9443
cargo run -p qt_server --example peer -- --relay ws://127.0.0.1:9443 --db alice.db
cargo run -p qt_server --example peer -- --relay ws://127.0.0.1:9443 --db bob.db
# in each one: /id shows the Public ID; /add <the other ID>; then type messages
```

`peer` uses the same protocol as the apps. `ws://` is only accepted for `localhost` in debug builds.

### Terminal messenger (talks to the phones)

```bash
cargo build --release -p qt_cli --bin messenger
export QT_RELAY_URL=wss://<your-relay>  QT_RELAY_PIN=<relay Noise public key>
M=./target/release/messenger
$M identity --db me.sqlite                 # creates the identity and shows the QT Public ID
$M add --db me.sqlite Ana qt1:...          # saves a contact
$M send --db me.sqlite --to Ana            # the message is asked for in the terminal
$M listen --db me.sqlite                   # keeps receiving (Ctrl+C to quit)
$M history --db me.sqlite Ana              # history, stored encrypted
$M remove --db me.sqlite Ana               # deletes the contact, the messages and the keys
```

The passphrase is always asked for hidden (or through `--passphrase-stdin`). Argon2id derives from it the keys for the database and the contact list.

### Hear a call on your PC (echo)

```bash
sudo apt install cmake libasound2-dev
cd tools/call-demo
cargo run --release -- eco              # microphone -> server -> headphones
cargo run --release -- eco --perda 15   # drops 15% of packets: hear DRED rebuilding them
cargo run --release -- eco --ws         # uses the WebSocket fallback
```

Use headphones (there is no echo cancellation on the PC).

### Android app

```bash
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/<version>
ABIS="arm64-v8a armeabi-v7a x86_64" ./scripts/build-android.sh   # libqt_ffi.so (16 KB) + Kotlin bindings
cd android && ./sync-native.sh                                    # copies into the app
# then: open the android/ folder in Android Studio
```

The *release* build needs its own keystore and the production server (`qtRelayUrl` / `qtRelayPin` in `android/gradle.properties`). Use NDK 30. Step by step in [`android/README.md`](android/README.md) and [`docs/android-integration.md`](docs/android-integration.md); API in [`docs/ffi-api.md`](docs/ffi-api.md).

### iPhone app

Needs a Mac with Xcode (the script installs Rust, [XcodeGen](https://github.com/yonaskolb/XcodeGen) and cmake if missing):

```bash
bash compilar-iphone.sh         # Rust core for iPhone and simulator, Xcode project, then opens it
```

Step by step in [`ios/README.md`](ios/README.md).

### Server on AWS

On a new EC2 instance with Ubuntu 24.04 (Security Group with 22, 80 and 443 TCP, and 443 UDP for calls):

```bash
curl -fsSLO https://raw.githubusercontent.com/moral666/QT/main/deploy/aws/setup-ec2.sh
sudo bash setup-ec2.sh
```

It installs Redis (only `127.0.0.1`, minimal ACL), the server and the call relay as isolated systemd services and HAProxy with a TLS certificate (certbot), and prints the `wss://` URL and the pinning key for the apps. To update, run the same script: it keeps the Noise key and the Redis password. It builds on the instance and needs disk space (the current server uses 20 GB). Details in [`docs/deploy-wss.md`](docs/deploy-wss.md). To join the relay to the federation: `/etc/qt/federacao.json` on both relays (section 25 of [`docs/protocol-spec.md`](docs/protocol-spec.md)).

## Documentation

- [`docs/protocol-spec.md`](docs/protocol-spec.md): how each piece works inside, byte by byte.
- [`docs/threat-model.md`](docs/threat-model.md): who this protects against, and who it does not.
- [`docs/known-limitations.md`](docs/known-limitations.md): known limitations, by impact.
- [`docs/field-tests.md`](docs/field-tests.md) and [`docs/performance.md`](docs/performance.md): tests with real devices and measurements.
- [`ROADMAP.md`](ROADMAP.md): what comes next. [`CHANGELOG.md`](CHANGELOG.md): what has changed.

Most documents in `docs/` are in Portuguese.

## License

Dual licence:

- **Open source:** GNU Affero General Public License, version 3 or later (AGPL-3.0-or-later). See [`LICENSE`](LICENSE).
- **Commercial:** for anyone who needs terms other than the AGPL, by written agreement with the author. See [`LICENSE-COMMERCIAL.md`](LICENSE-COMMERCIAL.md).

Third-party components keep their own licences.

## Contributing or reporting a problem

- [`CONTRIBUTING.md`](CONTRIBUTING.md): how to propose changes, with extra rules for cryptography code.
- [`SECURITY.md`](SECURITY.md): how to report a vulnerability privately (please do not open a public issue).
