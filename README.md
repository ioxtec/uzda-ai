# Uzda

Uzda is a learning project focused on Rust and the foundations of AI application development. Its long-term goal is to become a small harness for working with multiple AI models while keeping the application engine independent from its user interfaces.

The project is intentionally built in small steps. At this stage, it discovers local agent runtimes but does not start conversations, manage authentication, or call model APIs yet.

The workspace currently contains four crates:

- `uzda-core` — shared agent, session, and event types,
- `uzda-agent-codex` — the local Codex adapter,
- `uzda-agent-claude` — the local Claude Code adapter,
- `uzda-cli` — the command-line interface.

## Run

```sh
cargo run -p uzda-cli
```

List the agent runtimes available on your machine:

```sh
cargo run -p uzda-cli -- agents
```

Test the first real Codex app-server handshake:

```sh
cargo run -p uzda-cli -- codex status
```

## Check

```sh
cargo check
```

