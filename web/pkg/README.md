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
- **Wallet state** in localStorage (Cashu proofs and balances)
- **MDK state** in localStorage (OpenMLS group state and metadata)

## Features

- ✓ Generate/load Nostr keys
- ✓ MLS encrypted group messaging
- ✓ Multi-mint Cashu wallet
- ✓ Invite members to groups
- ✓ Send and receive e-cash tokens
- ✓ Trusted mint management
- ✓ Per-mint balance tracking
- ✓ Real-time message updates
- ✓ Mobile-responsive UI

## Known Issues / TODO

### MLS Commit Conflict Handling

**Problem:** When multiple users perform epoch-changing operations simultaneously (e.g., both trying to add a member), only one commit succeeds and the other is rejected. Currently, the web client doesn't handle this gracefully.

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

### Nostr Group ID Stability

**Current behavior:** The `nostr_group_id` field is randomly generated once at group creation and remains stable. All Kind 445 events (messages, commits) are tagged with `#h:<nostr_group_id>`, which allows efficient relay filtering.

**Implementation details:**
- `mls_group_id`: Never changes (primary key in storage)
- `nostr_group_id`: Used in event tags, has a setter method but no code calls it
- Current subscription filter: `kind:445` + `#h:<nostr_group_id>`

**Potential issue:** The storage type comment says `nostr_group_id` "can change over time" and a setter method exists (`set_nostr_group_id()`), suggesting this was designed to be mutable. However, no production code actually changes it.

**If `nostr_group_id` changes in the future:**
- Old messages would have `#h:<old_id>`
- New messages would have `#h:<new_id>`
- Current subscription would only see messages with the current ID
- Would need to track ID history and subscribe to all historical IDs, or remove the `#h` filter

**Recommendation:** Clarify whether `nostr_group_id` is intended to change. If yes, implement multi-ID subscription tracking. If no, mark it as immutable and remove the setter.
