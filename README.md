# MDK + Cashu Demo

A demonstration application combining **MDK (Marmot Development Kit)** (the implementation of MLS that is used in White Noise) for encrypted group messaging with **Cashu (CDK)** for ecash wallet functionality.

_**Built as part of the hackathon for the Bitcoin++ Lightning conference in Berlin in October 2025. This demo sets up three users in an MLS group, where they have the usual chat functionality. It's just a local process, with three 'fake' users messaging each other, but the messages use the MDK library to encrypt and sign the Nostr messages (kind 445 - MlsGroupMessage). Instead of using real public relays, those messages are just directly sent within the local process. This demo also includes a Cashu wallet (sqlite for the tokens) and users can use `!send 123` and `!redeemlast` and `!redeem cashuA...` commands to move cashu around in the group. Lots to do! Just a basic demo connecting `cdk` with `mdk`.**_

## Features

- **🔒 End-to-End Encrypted Group Chat**: Three users (Alice, Bob, Carol) communicate via MLS (Message Layer Security) protocol
- **💰 Cashu Wallet Integration**: Each user has a persistent SQLite-backed ecash wallet
- **🎁 Token Exchange**: Send and receive cashu tokens within the encrypted group chat
- **📡 Dual Mode**: Run in local simulation mode or connect to real Nostr relays
- **🖥️ Graphical Interface**: Three-pane GUI showing all users simultaneously
- **🔍 3x Zoom**: Default 300% zoom for easy demo presentations

## Architecture

- **MDK (Marmot Development Kit)**: Provides MLS encryption and Nostr relay integration
- **CDK (Cashu Development Kit)**: Handles ecash wallet operations and token management
- **egui/eframe**: Cross-platform GUI framework
- **SQLite**: Persistent wallet storage in `./wallets/` directory

## Prerequisites

- Rust 1.90.0 or later (specified in `rust-toolchain.toml`)
- Local clones of:
  - [MDK](https://github.com/parres-hq/mdk) in `./mdk/`
  - [CDK](https://github.com/cashubtc/cdk) in `./cdk/`

## Building

```bash
cargo build
```

## Running

### Local Simulation Mode (Default)
```bash
cargo run
```

### Nostr Relay Mode
```bash
cargo run -- --relay
```

Configured relays:
- wss://relay.damus.io
- wss://nos.lol
- wss://relay.nostr.band
- wss://relay.primal.net
- wss://nostr.bitcoiner.social
- wss://nostr.mom
- wss://nostr.oxtr.dev

## Usage

### GUI Controls

- **Zoom In/Out**: Buttons in the top bar (default 3x zoom)
- **Send Message**: Type in the input box and press Enter or click Send
- **Balance Display**: Shows current wallet balance at top of each pane

### Special Commands

#### `!topup [amount]`
Request a Lightning invoice to add sats to your wallet.
- Opens a QR code popup window
- Default: 100 sats if no amount specified
- Example: `!topup 50`

#### `!redeem <TOKEN>`
Receive a cashu token into your wallet.
- Token format: starts with `cashuA` or `cashuB`
- Example: `!redeem cashuAeyJ0b2tlbiI...`

#### `!send [amount]`
Create a cashu token and broadcast it to the group via MLS.
- Default: 10 sats if no amount specified
- Token appears formatted: `[🎁 Cashu Token: X sats from https://nofees.testnut.cashu.space]`
- Example: `!send 5`

#### `!redeemlast`
Automatically find and redeem the most recent cashu token from the chat.
- Searches backwards through messages
- Redeems the first cashu token found

## Mint Configuration

Currently using testnut mint (no real sats):
```
https://nofees.testnut.cashu.space
```

To change mint, edit `src/main.rs` line ~68.

## Wallet Storage

Wallets are persisted in SQLite databases:
- `./wallets/alice.db`
- `./wallets/bob.db`
- `./wallets/carol.db`

Balances are automatically restored on restart.

## Technical Details

### MLS Group Chat
- Uses OpenMLS via MDK for encryption
- Group ID is generated at startup
- In relay mode, messages are filtered by group ID using nostr `h` tag
- Forward secrecy and post-compromise security

### Cashu Integration
- Each wallet derived from user's Nostr keys
- Tokens are sent as encrypted MLS messages
- Two-step send process: `prepare_send()` → `confirm()`
- Automatic balance updates after transactions

### Message Flow (Local Mode)
1. User types message → MLS encryption
2. Encrypted message added to shared message queue
3. All users decrypt and display message

### Message Flow (Relay Mode)
1. User types message → MLS encryption
2. Published to Nostr relays as kind 445 event
3. Subscription filter matches group ID
4. All users receive, decrypt, and display

## Example Workflow

1. Start the app: `cargo run`
2. Bob creates a 5-sat token: `!send 5`
3. Alice redeems it: `!redeemlast`
4. Carol requests funds: `!topup 100`
5. (Pay the Lightning invoice shown in QR code)
6. Carol sends to Alice: `!send 20`

## License

See individual licenses for MDK and CDK dependencies.
