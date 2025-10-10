# MDK Ecash Web Client

A web-based application combining **MDK (Marmot Development Kit)** for encrypted group messaging with **Cashu (CDK)** for ecash wallet functionality.

Built around the hack days before and after the 2025 Bitcoin++ (Lightning) in Berlin. 

## Warnings

- I'm quite new to this, so don't trust your Bitcoin balance or secure comms to this hackday software 😀.
- Sometimes, a given group just seems to break down - such that users can't see each others messages. I guess it's something about a fork in the history of the epochs. Time to read more, and to understand how to recover.

## Features

- **🔒 End-to-End Encrypted Group Chat**: MLS (Message Layer Security) protocol via WebAssembly
- **💰 Cashu Wallet Integration**: Browser-based ecash wallet with localStorage persistence
- **🎁 Token Exchange**: Send and receive cashu tokens within encrypted group chat
- **📡 Nostr Integration**: Connect to real Nostr relays for message delivery
- **🌐 Browser-Based**: No installation required, runs entirely in the browser
- **👥 Multi-User**: Create groups, invite members, manage admins
- **📱 Modern UI**: Clean interface with real-time message updates

## Architecture

- **MDK (Marmot Development Kit)**: MLS encryption and Nostr relay integration (compiled to WASM)
- **CDK (Cashu Development Kit)**: Ecash wallet operations (compiled to WASM)
- **WebAssembly**: Rust code running in the browser
- **localStorage**: Persistent storage for Nostr keys, MDK state, and wallet data

## Prerequisites

- Rust 1.90.0 or later (specified in `rust-toolchain.toml`)
- wasm-pack: `cargo install wasm-pack`
- Local clones of:
  - [MDK](https://github.com/parres-hq/mdk) in `./mdk/`
  - [CDK](https://github.com/cashubtc/cdk) in `./cdk/`

## Building

```bash
cd web
wasm-pack build --target web --dev
```

This creates a `pkg/` directory with the compiled WASM module and JS glue code.

## Running

Start a local web server:

```bash
cd web
python3 -m http.server 4450
```

Then open http://localhost:4450 in your browser.

**Configured relays:**
- ws://localhost:8080 (local relay for development)
- wss://relay.damus.io
- wss://relay.primal.net

## Usage

### First Time Setup

1. Open the web app in your browser
2. Your Nostr keys will be automatically generated and stored in localStorage
3. Your npub (public key) is displayed at the top

### Creating a Group

1. Click "➕ Create New Group"
2. Enter group name and description
3. Provide the npub of at least one member to invite
4. Click "Create Group"

### Joining a Group

When someone invites you:
1. The app automatically detects the invitation
2. You'll see the group appear in your groups list
3. Click on the group to open the chat

### Sending Messages

- Type in the message box at the bottom
- Press Enter or click Send
- Messages are end-to-end encrypted using MLS

### Managing Groups

- **Invite Member**: Click "➕ Invite Member" button in chat
- All invited members are automatically promoted to admin

### Wallet

- **Receive e-cash**: Click "📥 Receive e-cash" button and paste a cashu token
- **Send e-cash**: Click "📤 Send e-cash" button, select a mint, and enter the amount
- **Manage Mints**: Add trusted mints, view balances per mint, set current mint for sending
- **Multi-mint Support**: Store tokens from multiple mints, with per-mint balance tracking

## Wallet Storage

Wallets are persisted in browser localStorage. Balances are automatically restored on page reload.

The default testnut mint is: `https://nofees.testnut.cashu.space` (no real sats).

## Technical Details

### MLS Group Chat
- Uses OpenMLS via MDK for encryption (compiled to WASM)
- Each group has a unique `nostr_group_id` for filtering
- Messages are filtered by group ID using nostr `#h` tag
- Forward secrecy and post-compromise security
- Subscription optimization: only fetches messages from last 10 minutes on subsequent opens

### Cashu Integration
- Wallet operations compiled to WebAssembly
- Multi-mint support with trust-based validation
- Automatic balance updates after transactions
- Per-mint balance tracking and management

### Message Flow
1. User types message → MLS encryption (in WASM)
2. Published to Nostr relays as kind 445 event with `#h:<group_id>` tag
3. Real-time subscription receives new events
4. WASM decrypts and displays message

### Storage Architecture
- **Nostr Keys**: localStorage (persistent)
- **MDK State**: localStorage via HybridStorage (OpenMLS state + group metadata)
- **Wallet State**: localStorage (Cashu proofs and balance)

### Group Events (Transparency)
All group operations generate visible messages:
- **Member joins**: "npub1abc... joined the group"
- **Admin promotion**: "npub1abc... promoted to admin"
- **Invitation record**: "Invited npub1abc... to the group (KeyPackage: xyz)"

## Example Workflow

1. Open the web app in your browser
2. Create a group and invite a friend (share your npub with them first)
3. Send messages in the encrypted group chat
4. Add a trusted mint in the Wallet section
5. Use "📤 Send e-cash" to create a cashu token and share it
6. Friend uses "📥 Receive e-cash" to claim the token

## License

See individual licenses for MDK and CDK dependencies.
