# mactop

A small Apple Silicon monitor with terminal and native menu-bar interfaces. CPU, memory, swap, and processes. No sudo required.

Personal project, shared as-is. No support or maintenance commitments.

![mactop with sample data](docs/screenshot.png)

## Run

Requires macOS on Apple Silicon and Rust.

```sh
cargo run --release
```

Press `?` for controls. Minimum terminal size is 90×25.

Live collection uses private Apple APIs; tested on an M4 Pro.

## Menu bar app

Requires macOS 14+, Apple Silicon, Rust, and Swift 6.2+ (Xcode or Command Line Tools).

```sh
./scripts/run-app
```

Separate CPU and MEM items open basic detail popovers. The native app shares the Rust collector with the TUI; UI design is still a work in progress. Build and architecture notes are in [development notes](docs/DEVELOPMENT.md#native-app).

## License

[MIT](LICENSE).

Inspired by [btop](https://github.com/aristocratos/btop) and [macmon](https://github.com/vladkens/macmon). See [third-party notices](THIRD_PARTY_NOTICES.md) for attribution and implementation references.
