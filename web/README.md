# MDK Ecash Web Client

Web-based client for MDK Ecash, compiled to WebAssembly.

## Prerequisites

Install wasm-pack:
```bash
cargo install wasm-pack
```

## Build

```bash
cd web
wasm-pack build --target web
```

This creates a `pkg/` directory with the compiled WASM module and JS glue code.

## Run

Start a local web server:

```bash
python3 -m http.server 4450
```

Then open http://localhost:4450 in your browser.

## Development

The web client stores:
- **Nostr keys** in localStorage (persistent across reloads)
- **Wallet state** will use IndexedDB (coming soon)

## Features

Current:
- ✓ Generate/load Nostr keys
- ✓ Display npub
- ✓ localStorage persistence

Coming soon:
- Cashu wallet integration
- MLS group messaging
- IndexedDB for wallet state
