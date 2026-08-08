# QT

**A technical prototype** of the cryptographic and transport core of an
E2EE messenger — X3DH, PQXDH (post-quantum), Double Ratchet, Sealed
Sender, Noise Protocol, all implemented in Rust, compiled and genuinely
tested. **This is not a finished product, not ready for real
conversations** — see "Be honest with me: does this actually work?"
below before spending time exploring it.

[Ler isto em português →](README.md)

## Context

This project grew out of a genuine interest in privacy and systems
security — specifically, the question of how to build communication
infrastructure that resists backdoors by design, not just by policy
(motivated, among other things, by laws like the EU's proposed "Chat
Control"). Rather than stopping at theory, the goal was to actually
implement and test every piece: not pseudocode, not diagrams — code that
compiles, runs, and has automated tests confirming it does what it claims.

## Why this exists (beyond the technical exercise)

Almost every popular messenger asks for your phone number to sign up.
Even when message content is private, that registration creates a
trail — who talks to whom, when, how often. There are also laws being
discussed in the European Union (the so-called "Chat Control") that could
force apps to scan message content, even in apps that claim to be
"encrypted."

QT tries to answer this with engineering, not just a privacy policy:
identity based on a cryptographic key (no phone, no email), end-to-end
encryption the server itself cannot break, and open source code so anyone
can verify there are no hidden tricks.

## Be honest with me: does this actually work?

Yes and no — and it's worth being clear about this before you invest time
exploring the project.

**Already working and genuinely tested** (not just code that "should
work"): end-to-end encryption, the network layer, a server that delivers
messages without ever being able to read them, an Android app that has
already run on a real device. All of this has automated tests passing,
and several parts were tested manually in the most rigorous way I could
manage (e.g.: killing the server mid-operation and confirming the data
survives).

**Not yet usable day-to-day**: the full user interface is missing (today
the Android app only has a "run demo" button), the server still needs to
be placed somewhere reachable over the internet, and an external security
audit is needed before anyone should trust this with real conversations.

In other words: the foundations are solid and tested; the building
doesn't have a roof yet.

## What's already built

| Piece | What it does | Confidence |
|---|---|---|
| `core/` | Session establishment (X3DH + post-quantum variant) and continuous encryption (Double Ratchet) — the same design used by Signal | ✅ Tested (14+ automated tests, including cases designed to fail) |
| `transport/` | Network connection protection (Noise Protocol) over WebSocket | ✅ Tested with real WebSocket connections, not simulated |
| `server/` | The "mailman": delivers messages without ever seeing the content or knowing who sent them | ✅ Tested, including surviving being force-killed (`kill -9`) |
| `storage/` | Stores your identity and conversations on disk, encrypted | ✅ Tested, including confirming a wrong password opens nothing |
| `cli/` | A terminal version, to try it out without a phone | ✅ Tested with completely separate processes talking to each other |
| `ffi/` | The bridge connecting all of this to Kotlin (Android), Swift (iOS) and Python | ✅ Tested from Python |
| Android app | Uses that bridge to prove everything works on a real device | ✅ Already ran on a real Android emulator |

## What's missing, plainly

- A real chat screen in the app (today it's just a demo button)
- A server reachable over the internet, not just on your local network
- TLS-protected connection (`wss://`) — currently uses plain `ws://`, fine for testing only
- Several independent servers collaborating (federation), so no single government can take down the whole network by shutting down just one
- The database password coming from the phone's security system (Keystore/Secure Enclave), not a command-line argument
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

### Using it as a persistent client (two people, separate processes)

```bash
cargo run --bin relay_server &

cargo run -p qt_cli --bin messenger -- identity --db bob.sqlite --passphrase "..."
cargo run -p qt_cli --bin messenger -- register --db bob.sqlite --passphrase "..." --server ws://127.0.0.1:9443

cargo run -p qt_cli --bin messenger -- identity --db alice.sqlite --passphrase "..."
cargo run -p qt_cli --bin messenger -- send --db alice.sqlite --passphrase "..." --to <BOB-ID> --message "Hi!" --server ws://127.0.0.1:9443

cargo run -p qt_cli --bin messenger -- receive --db bob.sqlite --passphrase "..." --server ws://127.0.0.1:9443
```

Each person's identifier is derived from their public key — nobody
chooses a "username," on purpose.

### Generating Android/iOS/Python bindings

```bash
./ffi/generate_bindings.sh
```

## How the project is organized

```
core/        The heart: end-to-end encryption, no external dependencies
transport/   Protects the network connection
server/      Delivers messages without being able to read them
storage/     Stores everything on disk, encrypted
cli/         A terminal client, for testing without a phone
demo/        Combines core+transport into a single test, to see it all in action
ffi/         The bridge to Kotlin, Swift and Python
docs/        The full technical specification, and the design history
```

Each piece has its own tests, kept next to the code (`tests/`). The
Android app exists and has already run on a real device (see the status
table above), but for now it only lives on the machine it was built on,
with no repository of its own and nothing published — treat it as a
technical proof of concept, not as something you can install yet. When
published, it should get its own repository that uses this one as a
dependency (the same separation principle explained in `CONTRIBUTING.md`).

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
