# Uzda

Uzda is a learning project focused on Rust and the foundations of AI application development. Its long-term goal is to become a small harness for working with multiple AI models while keeping the application engine independent from its user interfaces.

The project is intentionally built in small steps. At this stage, it does not include AI providers, networking libraries, or a complex architecture.

The project currently contains only two crates:

- `uzda-core` — reusable application logic,
- `uzda-cli` — a command-line interface that depends on `uzda-core`.

## Run

```sh
cargo run -p uzda-cli
```

## Check

```sh
cargo check
```

