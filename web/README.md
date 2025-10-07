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
- ✓ localStorage persistence for keys
- ✓ Placeholder wallet (shows mint URL and 0 balance)

Coming soon:
- Real CDK Cashu wallet integration (needs WASM-compatible storage)
- MLS group messaging
- IndexedDB for wallet state

## Known Issues / TODO

### MLS Commit Conflict Handling

**Problem:** When multiple users perform epoch-changing operations simultaneously (e.g., both trying to add a member), only one commit succeeds and the other is rejected. Currently, neither the web nor egui app handles this gracefully.

**Example scenario:**
- Alice and Bob both try to add Carol at epoch 5
- Both create commits: 5→6 (different commits)
- Alice publishes first → her commit accepted
- Bob publishes second → his commit rejected (already at epoch 6)
- Bob's client doesn't detect the conflict or retry

**Current behavior:**
- The "loser" (Bob) gets an error from `mdk.process_message()` when receiving the winning commit
- Error is logged but not handled
- Bob's state may diverge from the group

**Fix needed:**
- Detect commit conflicts (epoch mismatch errors)
- Re-sync state by processing missed commits
- Retry the operation if it still makes sense (e.g., if trying to add different members)
- Show user-friendly error if operation is no longer valid (e.g., member already added)

This is not critical for normal usage (commit conflicts are rare) but should be addressed for production.

## Notes

The CDK wallet integration is currently a placeholder because:
- SQLite doesn't compile to WASM (needs C stdlib)
- Need to implement IndexedDB-backed storage for wallet proofs
- Or use a simpler in-memory solution for the initial version
