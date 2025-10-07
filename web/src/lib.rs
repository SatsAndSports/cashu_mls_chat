use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use nostr::{Keys, ToBech32, FromBech32, EventBuilder, Kind, RelayUrl};
use nostr_sdk::{Client, RelayPoolNotification};
use web_sys::{window, Storage};
use std::sync::Arc;
use std::str::FromStr;
use std::time::Duration;
use serde::Serialize;

mod wallet_db;
use wallet_db::HybridWalletDatabase;

mod mdk_storage;
use mdk_storage::MdkHybridStorage;

use cdk::wallet::{Wallet, WalletBuilder, ReceiveOptions};
use cdk::nuts::{CurrencyUnit, Token};
use cdk::mint_url::MintUrl;

use mdk_core::MDK;
use mdk_storage_traits::GroupId;

// Relay URLs
const RELAYS: &[&str] = &[
    "ws://localhost:8080",
    "wss://relay.nostr.band",
    "wss://nostr.bitcoiner.social",
    "wss://nostr.land",
    "wss://relay.primal.net",
    "wss://nostr.strits.dk",
    "wss://nostr.oxtr.dev",
    "wss://nostr-pub.wellorder.net",
    "wss://orangesync.tech",
    "wss://relay.snort.social",
    "wss://vitor.nostr1.com",
    "wss://nostr.chaima.info",
    "wss://theforest.nostr1.com",
    "wss://articles.layer3.news",
    "wss://nostr-01.yakihonne.com",
    "wss://wot.nostr.party",
    "wss://no.str.cr",
    "wss://offchain.pub",
    "wss://primus.nostr1.com",
    "wss://bitcoiner.social",
    "wss://relay.jeffg.fyi",
];

/// Helper function to create MDK instance
async fn create_mdk() -> Result<MDK<MdkHybridStorage>, JsValue> {
    let storage = MdkHybridStorage::new().await?;
    Ok(MDK::new(storage))
}

/// Helper function to get Nostr keys
fn get_keys() -> Result<Keys, JsValue> {
    let storage = get_local_storage()?;
    let secret_hex = storage
        .get_item("nostr_secret_key")?
        .ok_or_else(|| JsValue::from_str("No keys found in localStorage"))?;

    Keys::parse(&secret_hex)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse keys: {}", e)))
}

/// Helper function to create a wallet from stored keys and database
async fn create_wallet() -> Result<Wallet, JsValue> {
    // Get Nostr keys from localStorage
    let storage = get_local_storage()?;
    let secret_hex = storage
        .get_item("nostr_secret_key")?
        .ok_or_else(|| JsValue::from_str("No keys found in localStorage"))?;

    let keys = Keys::parse(&secret_hex)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse keys: {}", e)))?;

    // Create seed from Nostr secret key
    let mut seed = [0u8; 64];
    seed[..32].copy_from_slice(keys.secret_key().as_secret_bytes());

    // Create mint URL
    let mint_url = MintUrl::from_str("https://nofees.testnut.cashu.space")
        .map_err(|e| JsValue::from_str(&format!("Invalid mint URL: {}", e)))?;

    // Create hybrid database (loads from localStorage)
    let db = HybridWalletDatabase::new().await?;

    // Build wallet
    let wallet = WalletBuilder::new()
        .mint_url(mint_url)
        .unit(CurrencyUnit::Sat)
        .localstore(Arc::new(db))
        .seed(seed)
        .build()
        .map_err(|e| JsValue::from_str(&format!("Failed to build wallet: {}", e)))?;

    Ok(wallet)
}

// Helper to get localStorage
fn get_local_storage() -> Result<Storage, JsValue> {
    window()
        .ok_or_else(|| JsValue::from_str("No window object"))?
        .local_storage()?
        .ok_or_else(|| JsValue::from_str("No localStorage available"))
}

/// Initialize the library (call this first from JavaScript)
#[wasm_bindgen(start)]
pub fn init() {
    // Set panic hook for better error messages in console
    console_error_panic_hook::set_once();
}

/// Generate new Nostr keys and save to localStorage
/// Also clears MDK state since it's associated with the old identity (keeps wallet)
#[wasm_bindgen]
pub fn generate_keys() -> Result<String, JsValue> {
    let keys = Keys::generate();
    let secret_hex = keys.secret_key().to_secret_hex();

    let storage = get_local_storage()?;

    // Clear MDK state from previous identity (but keep wallet)
    storage.remove_item("mdk_state")?;
    log("Cleared old MDK state for fresh start (wallet preserved)");

    // Save new keys
    storage.set_item("nostr_secret_key", &secret_hex)?;

    Ok(keys.public_key().to_bech32().expect("bech32 encoding is infallible"))
}

/// Load existing keys from localStorage, or generate if none exist
#[wasm_bindgen]
pub fn get_or_create_keys() -> Result<String, JsValue> {
    let storage = get_local_storage()?;

    if let Some(secret_hex) = storage.get_item("nostr_secret_key")? {
        // Load existing keys
        let keys = Keys::parse(&secret_hex)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse keys: {}", e)))?;
        Ok(keys.public_key().to_bech32().expect("bech32 encoding is infallible"))
    } else {
        // Generate new keys
        generate_keys()
    }
}

/// Get the npub (public key in bech32 format)
#[wasm_bindgen]
pub fn get_npub() -> Result<String, JsValue> {
    let storage = get_local_storage()?;

    let secret_hex = storage
        .get_item("nostr_secret_key")?
        .ok_or_else(|| JsValue::from_str("No keys found in localStorage"))?;

    let keys = Keys::parse(&secret_hex)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse keys: {}", e)))?;

    Ok(keys.public_key().to_bech32().expect("bech32 encoding is infallible"))
}

/// Clear all stored keys and MDK state (for testing, keeps wallet)
#[wasm_bindgen]
pub fn clear_keys() -> Result<(), JsValue> {
    let storage = get_local_storage()?;
    storage.remove_item("nostr_secret_key")?;
    storage.remove_item("mdk_state")?;
    log("Cleared Nostr keys and MDK state (wallet preserved)");
    Ok(())
}

/// Log to browser console (for debugging)
#[wasm_bindgen]
pub fn log(message: &str) {
    web_sys::console::log_1(&JsValue::from_str(message));
}

/// Initialize a real CDK wallet with hybrid in-memory + localStorage storage
/// Returns a Promise that resolves to the initial balance
#[wasm_bindgen]
pub fn init_wallet() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("Initializing CDK wallet with hybrid storage...");

            // Create wallet
            let wallet = create_wallet().await?;

            log("Fetching wallet balance...");

            // Get initial balance
            let balance = wallet
                .total_balance()
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to get balance: {}", e)))?;

            log(&format!("✅ Wallet initialized! Balance: {} sats", balance));

            Ok::<u64, JsValue>(u64::from(balance))
        }
        .await;

        result.map(|b| JsValue::from_f64(b as f64))
    })
}

/// Get wallet balance (recreates wallet from localStorage each time)
/// Returns a Promise that resolves to the current balance
#[wasm_bindgen]
pub fn get_balance() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("Fetching balance from wallet...");

            // Create wallet (loads from localStorage)
            let wallet = create_wallet().await?;

            // Get balance
            let balance = wallet
                .total_balance()
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to get balance: {}", e)))?;

            log(&format!("Balance: {} sats", balance));

            Ok::<u64, JsValue>(u64::from(balance))
        }
        .await;

        result.map(|b| JsValue::from_f64(b as f64))
    })
}

/// Parse token information without receiving it
/// Returns a Promise that resolves to JSON with token info
#[wasm_bindgen]
pub fn parse_token_info(token_str: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            // Parse token
            let token = Token::from_str(&token_str)
                .map_err(|e| JsValue::from_str(&format!("Invalid token: {}", e)))?;

            // Get total amount
            let amount = token.value()
                .map_err(|e| JsValue::from_str(&format!("Failed to get value: {}", e)))?;

            // Get mint URL
            let mint_url = token.mint_url()
                .map_err(|e| JsValue::from_str(&format!("Failed to get mint URL: {}", e)))?;

            // Create JSON response
            #[derive(Serialize)]
            struct TokenInfo {
                amount: u64,
                mint: String,
            }

            let info = TokenInfo {
                amount: u64::from(amount),
                mint: mint_url.to_string(),
            };

            let json = serde_json::to_string(&info)
                .map_err(|e| JsValue::from_str(&format!("Failed to serialize: {}", e)))?;

            Ok::<String, JsValue>(json)
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Receive ecash token
/// Returns a Promise that resolves to the amount received
#[wasm_bindgen]
pub fn receive_token(token_str: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("Receiving token: {}", &token_str[..20.min(token_str.len())]));

            // Parse token to validate it
            let _token = Token::from_str(&token_str)
                .map_err(|e| JsValue::from_str(&format!("Invalid token: {}", e)))?;

            log(&format!("Token parsed, attempting to receive..."));

            // Create wallet (loads from localStorage)
            let wallet = create_wallet().await?;

            // Receive the token
            let amount = wallet
                .receive(&token_str, ReceiveOptions::default())
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to receive token: {}", e)))?;

            log(&format!("✅ Received {} sats!", amount));

            Ok::<u64, JsValue>(u64::from(amount))
        }
        .await;

        result.map(|amount| JsValue::from_f64(amount as f64))
    })
}

/// Get all groups from MDK storage
/// Returns a Promise that resolves to a JSON array of groups
#[wasm_bindgen]
pub fn get_groups() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("Fetching groups from MDK...");

            let mdk = create_mdk().await?;
            let groups = mdk
                .get_groups()
                .map_err(|e| JsValue::from_str(&format!("Failed to get groups: {}", e)))?;

            log(&format!("Found {} group(s)", groups.len()));

            // Convert to JSON array with member count
            let groups_json: Vec<_> = groups.iter().map(|g| {
                // Get member count
                let member_count = mdk.get_members(&g.mls_group_id)
                    .ok()
                    .map(|members| members.len())
                    .unwrap_or(0);

                serde_json::json!({
                    "id": hex::encode(g.mls_group_id.as_slice()),
                    "name": g.name,
                    "description": g.description,
                    "image_hash": g.image_hash.map(|h| hex::encode(h)),
                    "last_message_at": g.last_message_at.map(|t| t.as_u64()),
                    "member_count": member_count,
                })
            }).collect();

            let json = serde_json::to_string(&groups_json)
                .map_err(|e| JsValue::from_str(&format!("Failed to serialize: {}", e)))?;

            Ok::<String, JsValue>(json)
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Fetch Welcome events from Nostr relays and process them with MDK
/// Returns a Promise that resolves to the number of Welcome events processed
#[wasm_bindgen]
pub fn fetch_welcome_events() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("Fetching Welcome events from Nostr relays...");

            // Get our keys
            let keys = get_keys()?;
            let pubkey = keys.public_key();

            // Create client and connect to relays
            let client = Client::default();
            for relay in RELAYS {
                if let Ok(url) = RelayUrl::parse(relay) {
                    let _ = client.add_relay(url).await;
                }
            }
            client.connect().await;

            // Step 1: Get our KeyPackage event IDs that we have private keys for
            log("Finding our KeyPackage events...");
            let kp_filter = nostr::Filter::new()
                .kind(Kind::Custom(443))
                .author(pubkey);

            let kp_events = client.fetch_events(kp_filter, Duration::from_secs(5)).await
                .map_err(|e| JsValue::from_str(&format!("Failed to fetch KeyPackages: {}", e)))?;

            let kp_event_ids: Vec<String> = kp_events.iter().map(|e| e.id.to_hex()).collect();
            log(&format!("Found {} KeyPackage(s) on relays: {:?}", kp_event_ids.len(), kp_event_ids));

            if kp_event_ids.is_empty() {
                log("⚠️ No KeyPackages found. Create a new KeyPackage to receive invites.");
                return Ok::<u32, JsValue>(0);
            }

            // Step 2: Get Welcome events that reference our KeyPackages
            log("Querying relays for Welcome events...");

            let filter = nostr::Filter::new()
                .kind(Kind::Custom(444));

            let all_welcomes = client.fetch_events(filter, Duration::from_secs(10)).await
                .map_err(|e| JsValue::from_str(&format!("Failed to fetch Welcome events: {}", e)))?;

            let total_welcomes = all_welcomes.len();

            // Filter to only Welcomes that reference our KeyPackages
            let events: Vec<_> = all_welcomes.into_iter().filter(|event| {
                event.tags.iter().any(|tag| {
                    let kind = tag.kind();
                    if kind.as_str() == "e" {
                        if let Some(event_id) = tag.content() {
                            return kp_event_ids.iter().any(|kp_id| kp_id == event_id);
                        }
                    }
                    false
                })
            }).collect();

            log(&format!("Found {} Welcome event(s) for us (out of {} total)", events.len(), total_welcomes));

            // Disconnect from relays
            let _ = client.disconnect().await;

            // Process each Welcome event with MDK
            let mdk = create_mdk().await?;
            let mut processed = 0;

            for event in events {
                log(&format!("Processing Welcome event: {}", event.id.to_hex()));

                // Log which KeyPackage this Welcome references
                let referenced_kp: Vec<String> = event.tags.iter()
                    .filter(|tag| tag.kind().as_str() == "e")
                    .filter_map(|tag| tag.content().map(|s| s.to_string()))
                    .collect();
                log(&format!("  References KeyPackage(s): {:?}", referenced_kp));

                // Log which pubkeys this Welcome is addressed to (p tags)
                let addressed_to: Vec<String> = event.tags.iter()
                    .filter(|tag| tag.kind().as_str() == "p")
                    .filter_map(|tag| tag.content())
                    .filter_map(|pk_hex| {
                        nostr::PublicKey::from_hex(pk_hex)
                            .ok()
                            .and_then(|pk| pk.to_bech32().ok())
                    })
                    .collect();
                if !addressed_to.is_empty() {
                    log(&format!("  Addressed to npub(s): {:?}", addressed_to));
                } else {
                    log("  No p tags found");
                }

                log(&format!("  Our npub: {}", pubkey.to_bech32().expect("valid bech32")));

                // Check if we already have this welcome in storage
                match mdk.get_welcome(&event.id) {
                    Ok(Some(_)) => {
                        log("  ⚠ Welcome already in storage, skipping");
                        continue;
                    }
                    Ok(None) => {
                        // Not in storage yet, proceed
                    }
                    Err(e) => {
                        log(&format!("  ⚠ Error checking welcome storage: {}", e));
                        continue;
                    }
                }

                // Try to extract the rumor from the event
                // Welcome events might be gift-wrapped (kind 1059) or direct (kind 444)
                match nostr::nips::nip59::UnwrappedGift::from_gift_wrap(&keys, &event).await {
                    Ok(unwrapped) => {
                        // Successfully unwrapped - process the rumor
                        log("  Unwrapped gift-wrapped Welcome");
                        match mdk.process_welcome(&event.id, &unwrapped.rumor) {
                            Ok(_) => {
                                log("  ✓ Welcome processed successfully");
                                processed += 1;
                            }
                            Err(e) => {
                                log(&format!("  ✗ Failed to process Welcome: {}", e));
                            }
                        }
                    }
                    Err(_) => {
                        // Not gift-wrapped - process as direct kind 444 event
                        log("  Processing as direct (non-gift-wrapped) Welcome");

                        // Log event details for debugging
                        log(&format!("    Event author: {}", event.pubkey.to_hex()));
                        log(&format!("    Event kind: {}", event.kind.as_u16()));
                        log(&format!("    Event tags: {:?}", event.tags.len()));
                        log(&format!("    Content length: {} bytes", event.content.len()));
                        log(&format!("    Content preview: {}...", &event.content[..event.content.len().min(100)]));

                        // Convert Event to UnsignedEvent
                        let rumor = nostr::UnsignedEvent {
                            id: None,
                            pubkey: event.pubkey,
                            created_at: event.created_at,
                            kind: event.kind,
                            tags: event.tags.clone(),
                            content: event.content.clone(),
                        };

                        log("  Calling MDK.process_welcome()...");
                        match mdk.process_welcome(&event.id, &rumor) {
                            Ok(_) => {
                                log("  ✓ Welcome processed successfully");
                                processed += 1;
                            }
                            Err(e) => {
                                log(&format!("  ✗ Failed to process Welcome: {}", e));
                            }
                        }
                    }
                }
            }

            log(&format!("✅ Processed {} Welcome event(s)", processed));

            Ok::<u32, JsValue>(processed)
        }
        .await;

        result.map(|count| JsValue::from_f64(count as f64))
    })
}

/// Generate KeyPackage and wait for group invite (all in one flow)
/// Returns a Promise that resolves to the group name when joined
#[wasm_bindgen]
pub fn create_keypackage_and_wait_for_invite() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("🔑 Creating KeyPackage and waiting for invite...");

            // Get keys
            let keys = get_keys()?;
            let pubkey = keys.public_key();

            // Create MDK (this instance will stay alive for the whole flow)
            let mdk = create_mdk().await?;

            // Step 1: Create KeyPackage
            log("Creating KeyPackage...");
            let relay_urls: Vec<RelayUrl> = RELAYS
                .iter()
                .filter_map(|r| RelayUrl::parse(r).ok())
                .collect();

            let (key_package_hex, tags) = mdk
                .create_key_package_for_event(&pubkey, relay_urls)
                .map_err(|e| JsValue::from_str(&format!("Failed to create KeyPackage: {}", e)))?;

            // Step 2: Build and sign event
            let event = EventBuilder::new(Kind::Custom(443), key_package_hex)
                .tags(tags.to_vec())
                .sign_with_keys(&keys)
                .map_err(|e| JsValue::from_str(&format!("Failed to sign event: {}", e)))?;

            let kp_event_id = event.id;
            log(&format!("KeyPackage event ID: {}", kp_event_id.to_hex()));

            // Step 3: Connect to relays and start listening for Welcomes BEFORE publishing
            let client = Client::default();
            for relay in RELAYS {
                if let Ok(url) = RelayUrl::parse(relay) {
                    let _ = client.add_relay(url).await;
                }
            }
            client.connect().await;

            // Subscribe to Welcomes that reference our KeyPackage
            log("Starting Welcome subscription...");
            let filter = nostr::Filter::new()
                .kind(Kind::Custom(444))
                .since(nostr::Timestamp::now());  // Only new Welcomes from now on

            client.subscribe(filter, None).await
                .map_err(|e| JsValue::from_str(&format!("Failed to subscribe: {}", e)))?;

            // Step 4: Now publish KeyPackage (Welcomes can arrive while publishing!)
            log("Publishing KeyPackage to relays...");
            let send_result = client.send_event(&event).await
                .map_err(|e| JsValue::from_str(&format!("Failed to publish: {}", e)))?;

            log(&format!("✅ KeyPackage published! Your npub: {}", pubkey.to_bech32().expect("valid bech32")));
            for relay_url in send_result.success.iter() {
                log(&format!("  ✓ {} accepted", relay_url));
            }
            for (relay_url, error) in send_result.failed.iter() {
                log(&format!("  ✗ {} rejected: {}", relay_url, error));
            }
            log("⏳ Waiting for group invite...");

            // Step 5: Listen for Welcome via subscription (may already be here!)
            let mut notifications = client.notifications();
            log("Listening for Welcome messages...");

            loop {
                // Wait for next notification from subscription
                match notifications.recv().await {
                    Ok(notification) => {
                        if let nostr_sdk::RelayPoolNotification::Event { event: welcome_event, .. } = notification {
                            log(&format!("📩 Received event: {}", welcome_event.id.to_hex()));

                            // Check if this Welcome references our KeyPackage
                            let references_our_kp = welcome_event.tags.iter().any(|tag| {
                                tag.kind().as_str() == "e" &&
                                tag.content().map(|c| c == kp_event_id.to_hex()).unwrap_or(false)
                            });

                            if !references_our_kp {
                                log("  Not for our KeyPackage, continuing...");
                                continue;
                            }

                            log("  ✅ This Welcome is for our KeyPackage!");

                            // Process the Welcome
                            log(&format!("Processing Welcome: {}", welcome_event.id.to_hex()));

                            // Convert to UnsignedEvent (compute the ID)
                            let mut rumor = nostr::UnsignedEvent {
                                id: None,
                                pubkey: welcome_event.pubkey,
                                created_at: welcome_event.created_at,
                                kind: welcome_event.kind,
                                tags: welcome_event.tags.clone(),
                                content: welcome_event.content.clone(),
                            };
                            // Ensure the ID is set
                            let _ = rumor.id();

                            // Process Welcome
                            let welcome = mdk.process_welcome(&welcome_event.id, &rumor)
                                .map_err(|e| JsValue::from_str(&format!("Failed to process Welcome: {}", e)))?;

                            log(&format!("Welcome processed! Group: {}", welcome.group_name));

                            // Accept Welcome (join the group)
                            mdk.accept_welcome(&welcome)
                                .map_err(|e| JsValue::from_str(&format!("Failed to accept Welcome: {}", e)))?;

                            log(&format!("✅ Joined group: {}", welcome.group_name));

                            // Send a "hi" message to the group
                            log("Sending greeting message...");
                            let greeting_rumor = nostr::UnsignedEvent {
                                id: None,
                                pubkey,
                                created_at: nostr::Timestamp::now(),
                                kind: Kind::GiftWrap,  // Use GiftWrap kind for MLS messages
                                tags: nostr::Tags::new(),
                                content: "Hi everyone! 👋".to_string(),
                            };

                            let message_event = mdk.create_message(&welcome.mls_group_id, greeting_rumor)
                                .map_err(|e| JsValue::from_str(&format!("Failed to create message: {}", e)))?;

                            // Publish greeting message
                            client.send_event(&message_event).await
                                .map_err(|e| JsValue::from_str(&format!("Failed to send message: {}", e)))?;

                            log("✅ Greeting sent!");

                            // Disconnect and return
                            let _ = client.disconnect().await;

                            return Ok::<String, JsValue>(welcome.group_name);
                        }
                    }
                    Err(e) => {
                        log(&format!("Error receiving notification: {:?}", e));
                        // Continue waiting
                    }
                }
            }
        }
        .await;

        result.map(|name| JsValue::from_str(&name))
    })
}

/// Create a new group and invite members
/// Returns a Promise that resolves to the group ID
#[wasm_bindgen]
pub fn create_group_with_members(name: String, description: String, member_npubs_json: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("📝 Creating group: {}", name));

            // Parse member npubs
            let member_npubs: Vec<String> = serde_json::from_str(&member_npubs_json)
                .map_err(|e| JsValue::from_str(&format!("Invalid npubs JSON: {}", e)))?;

            // Get our keys
            let keys = get_keys()?;
            let our_pubkey = keys.public_key();

            // Fetch KeyPackages for each member
            log(&format!("Fetching KeyPackages for {} member(s)...", member_npubs.len()));

            let client = Client::default();
            for relay in RELAYS {
                if let Ok(url) = RelayUrl::parse(relay) {
                    let _ = client.add_relay(url).await;
                }
            }
            client.connect().await;

            let mut key_package_events = Vec::new();
            let mut admin_pubkeys = vec![our_pubkey]; // Creator is always admin

            for npub in &member_npubs {
                log(&format!("  Fetching KeyPackage for {}...", &npub[..16]));

                // Parse npub to get public key
                let pubkey = nostr::PublicKey::from_bech32(npub)
                    .map_err(|e| JsValue::from_str(&format!("Invalid npub {}: {}", npub, e)))?;

                admin_pubkeys.push(pubkey);

                // Query for their most recent KeyPackage (kind 443)
                let filter = nostr::Filter::new()
                    .kind(Kind::Custom(443))
                    .author(pubkey)
                    .limit(10);  // Get last 10, we'll pick the newest

                let events = client.fetch_events(filter, Duration::from_secs(10)).await
                    .map_err(|e| JsValue::from_str(&format!("Failed to fetch KeyPackages for {}: {}", npub, e)))?;

                if events.is_empty() {
                    return Err(JsValue::from_str(&format!("No KeyPackage found for {}", npub)));
                }

                // Get the newest KeyPackage
                let newest = events.iter()
                    .max_by_key(|e| e.created_at)
                    .unwrap();

                log(&format!("    ✓ Found KeyPackage: {}", newest.id.to_hex()));
                key_package_events.push(newest.clone());
            }

            // Create group config
            use mdk_core::prelude::*;
            let relay_urls: Vec<RelayUrl> = RELAYS
                .iter()
                .filter_map(|r| RelayUrl::parse(r).ok())
                .collect();

            let config = NostrGroupConfigData::new(
                name.clone(),
                description,
                None,  // image
                None,  // banner
                None,  // website
                relay_urls,
                admin_pubkeys,
            );

            // Create MDK and create the group
            log("Creating group with MDK...");
            let mdk = create_mdk().await?;

            let group_result = mdk.create_group(&our_pubkey, key_package_events, config)
                .map_err(|e| JsValue::from_str(&format!("Failed to create group: {}", e)))?;

            let group_id = hex::encode(group_result.group.mls_group_id.as_slice());
            log(&format!("✅ Group created! ID: {}", &group_id[..16]));

            // Publish Welcome messages to each invited member
            log(&format!("Publishing {} Welcome message(s)...", group_result.welcome_rumors.len()));

            for welcome_unsigned in group_result.welcome_rumors {
                // Sign the UnsignedEvent
                let welcome_event = welcome_unsigned.sign(&keys).await
                    .map_err(|e| JsValue::from_str(&format!("Failed to sign Welcome: {}", e)))?;

                client.send_event(&welcome_event).await
                    .map_err(|e| JsValue::from_str(&format!("Failed to send Welcome: {}", e)))?;
            }

            log(&format!("✅ All Welcome messages published!"));

            // Disconnect
            let _ = client.disconnect().await;

            Ok::<String, JsValue>(group_id)
        }
        .await;

        result.map(|group_id| JsValue::from_str(&group_id))
    })
}

/// Invite a member to an existing group by their npub
/// Returns a Promise that resolves when the invite is sent
#[wasm_bindgen]
pub fn invite_member_to_group(group_id_hex: String, member_npub: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("👋 Inviting {} to group {}", &member_npub[..16], &group_id_hex[..16]));

            // Parse npub to get public key
            let member_pubkey = nostr::PublicKey::from_bech32(&member_npub)
                .map_err(|e| JsValue::from_str(&format!("Invalid npub: {}", e)))?;

            // Parse group ID
            let group_id_bytes = hex::decode(&group_id_hex)
                .map_err(|e| JsValue::from_str(&format!("Invalid group ID: {}", e)))?;
            let group_id = mdk_core::prelude::GroupId::from_slice(&group_id_bytes);

            // Fetch member's KeyPackage
            log(&format!("Fetching KeyPackage for {}...", &member_npub[..16]));

            let client = Client::default();
            for relay in RELAYS {
                if let Ok(url) = RelayUrl::parse(relay) {
                    let _ = client.add_relay(url).await;
                }
            }
            client.connect().await;

            let filter = nostr::Filter::new()
                .kind(Kind::Custom(443))
                .author(member_pubkey)
                .limit(10);

            let events = client.fetch_events(filter, Duration::from_secs(10)).await
                .map_err(|e| JsValue::from_str(&format!("Failed to fetch KeyPackage: {}", e)))?;

            if events.is_empty() {
                return Err(JsValue::from_str(&format!("No KeyPackage found for {}. They may need to create one first.", &member_npub[..16])));
            }

            // Get the newest KeyPackage
            let newest = events.iter()
                .max_by_key(|e| e.created_at)
                .unwrap();

            log(&format!("  ✓ Found KeyPackage: {}", newest.id.to_hex()));

            // Get our keys
            let keys = get_keys()?;

            // Create MDK and add member to group
            log("Adding member to group...");
            let mdk = create_mdk().await?;

            let invite_result = mdk.add_members(&group_id, &[newest.clone()])
                .map_err(|e| JsValue::from_str(&format!("Failed to add member: {}", e)))?;

            // Publish Welcome message if any
            if let Some(welcome_rumors) = invite_result.welcome_rumors {
                log(&format!("Publishing Welcome message to {}...", &member_npub[..16]));

                for welcome_unsigned in welcome_rumors {
                    // Sign the UnsignedEvent
                    let welcome_event = welcome_unsigned.sign(&keys).await
                        .map_err(|e| JsValue::from_str(&format!("Failed to sign Welcome: {}", e)))?;

                    client.send_event(&welcome_event).await
                        .map_err(|e| JsValue::from_str(&format!("Failed to send Welcome: {}", e)))?;
                }

                log(&format!("✅ Invite sent to {}!", &member_npub[..16]));
            } else {
                log(&format!("✅ Member added to group (no Welcome needed)"));
            }

            // Disconnect
            let _ = client.disconnect().await;

            Ok::<JsValue, JsValue>(JsValue::from_str("success"))
        }
        .await;

        result
    })
}

/// Check for and auto-accept any pending Welcome messages from MDK storage
/// Returns a Promise that resolves to the number of groups joined
#[wasm_bindgen]
pub fn process_pending_welcomes() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("Checking for pending Welcome messages...");

            // Get MDK instance
            let mdk = create_mdk().await?;

            // Get all pending welcomes and auto-accept them
            let pending_welcomes = mdk.get_pending_welcomes()
                .map_err(|e| JsValue::from_str(&format!("Failed to get pending welcomes: {}", e)))?;

            log(&format!("Found {} pending welcome(s) to accept", pending_welcomes.len()));

            let mut accepted = 0;
            for welcome in pending_welcomes {
                log(&format!("Auto-accepting welcome for group: {}", hex::encode(welcome.mls_group_id.as_slice())));

                match mdk.accept_welcome(&welcome) {
                    Ok(_) => {
                        log("  ✓ Welcome accepted, joined group!");
                        accepted += 1;
                    }
                    Err(e) => {
                        log(&format!("  ✗ Failed to accept: {}", e));
                    }
                }
            }

            if accepted > 0 {
                log(&format!("✅ Joined {} new group(s)", accepted));
            } else {
                log("No new groups to join");
            }

            Ok::<u32, JsValue>(accepted)
        }
        .await;

        result.map(|count| JsValue::from_f64(count as f64))
    })
}

/// Send a message to a group
/// Returns a Promise that resolves when the message is sent
#[wasm_bindgen]
pub fn send_message_to_group(group_id_hex: String, message_content: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("📤 Sending message to group {}", &group_id_hex[..16]));
            log(&format!("  Message content: {}", message_content));

            // Get keys
            let keys = get_keys()?;
            let pubkey = keys.public_key();
            log(&format!("  Sender npub: {}", pubkey.to_bech32().expect("valid bech32")));

            // Decode group ID from hex
            let group_id_bytes = hex::decode(&group_id_hex)
                .map_err(|e| JsValue::from_str(&format!("Invalid group ID hex: {}", e)))?;
            let group_id = GroupId::from_slice(&group_id_bytes);
            log(&format!("  Decoded group ID: {} bytes", group_id_bytes.len()));

            // Create MDK
            log("  Creating MDK instance...");
            let mdk = create_mdk().await?;
            log("  ✓ MDK instance created");

            // Verify group exists
            log("  Checking if group exists...");
            let group = mdk.get_group(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to get group: {}", e)))?
                .ok_or_else(|| JsValue::from_str("Group not found"))?;
            log(&format!("  ✓ Group found: {}", &group.name));

            // Create message rumor
            log("  Creating message rumor...");
            let rumor = nostr::UnsignedEvent {
                id: None,
                pubkey,
                created_at: nostr::Timestamp::now(),
                kind: Kind::GiftWrap,
                tags: nostr::Tags::new(),
                content: message_content.clone(),
            };
            log("  ✓ Message rumor created");

            // Create encrypted message
            log("  Encrypting message with MLS...");
            let message_event = mdk.create_message(&group_id, rumor)
                .map_err(|e| JsValue::from_str(&format!("Failed to create message: {}", e)))?;
            log(&format!("  ✓ Message encrypted, event ID: {}", message_event.id.to_hex()));

            // Publish to relays
            log("  Connecting to relays...");
            let client = Client::default();
            for relay in RELAYS {
                if let Ok(url) = RelayUrl::parse(relay) {
                    log(&format!("    Adding relay: {}", relay));
                    let _ = client.add_relay(url).await;
                }
            }
            client.connect().await;
            log("  ✓ Connected to relays");

            log("  Publishing message event...");
            client.send_event(&message_event).await
                .map_err(|e| JsValue::from_str(&format!("Failed to send event: {}", e)))?;
            log("  ✓ Message event published");

            // Disconnect
            let _ = client.disconnect().await;
            log("✅ Message sent successfully!");

            Ok::<(), JsValue>(())
        }
        .await;

        result.map(|_| JsValue::undefined())
    })
}

/// Get messages for a group from storage
/// Returns a Promise that resolves to a JSON array of messages
#[wasm_bindgen]
pub fn get_messages_for_group(group_id_hex: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            // Decode group ID from hex
            let group_id_bytes = hex::decode(&group_id_hex)
                .map_err(|e| JsValue::from_str(&format!("Invalid group ID hex: {}", e)))?;
            let group_id = GroupId::from_slice(&group_id_bytes);

            // Get storage
            let storage = MdkHybridStorage::new().await?;

            // Get messages using the GroupStorage trait
            use mdk_storage_traits::groups::GroupStorage;
            let messages = storage.messages(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to get messages: {}", e)))?;

            // Convert messages to JSON
            #[derive(Serialize)]
            struct MessageJson {
                id: String,
                pubkey: String,
                content: String,
                created_at: u64,
                state: String,
            }

            let messages_json: Vec<MessageJson> = messages.iter().map(|msg| {
                MessageJson {
                    id: msg.id.to_hex(),
                    pubkey: msg.pubkey.to_hex(),
                    content: msg.content.clone(),
                    created_at: msg.created_at.as_u64(),
                    state: msg.state.to_string(),
                }
            }).collect();

            let json = serde_json::to_string(&messages_json)
                .map_err(|e| JsValue::from_str(&format!("Failed to serialize: {}", e)))?;

            Ok::<String, JsValue>(json)
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Message JSON structure for JavaScript callback
#[derive(Serialize)]
struct MessageCallback {
    id: String,
    pubkey: String,
    content: String,
    created_at: u64,
    state: String,
}

/// Subscribe to group messages and call a JavaScript callback for each new message
/// The callback will receive a JSON object with message details
#[wasm_bindgen]
pub fn subscribe_to_group_messages(group_id_hex: String, callback: js_sys::Function) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("📡 Subscribing to messages for group {}", &group_id_hex[..16]));

            // Decode group ID
            let group_id_bytes = hex::decode(&group_id_hex)
                .map_err(|e| JsValue::from_str(&format!("Invalid group ID hex: {}", e)))?;
            let group_id = GroupId::from_slice(&group_id_bytes);

            // Create client and connect to relays
            let client = Client::default();
            for relay in RELAYS {
                if let Ok(url) = RelayUrl::parse(relay) {
                    log(&format!("  Adding relay: {}", relay));
                    let _ = client.add_relay(url).await;
                }
            }
            client.connect().await;
            log("  ✓ Connected to relays");

            // Subscribe to MLS group messages (kind 445) from recent history
            let now = nostr::Timestamp::now();
            let recent = nostr::Timestamp::from(now.as_u64().saturating_sub(10));
            let filter = nostr::Filter::new()
                .kind(Kind::MlsGroupMessage)
                .since(recent);

            log("  Subscribing to MLS group messages (kind 445)...");
            client.subscribe(filter, None).await
                .map_err(|e| JsValue::from_str(&format!("Failed to subscribe: {}", e)))?;
            log("  ✓ Subscribed successfully");

            // Spawn a background task to listen for notifications
            wasm_bindgen_futures::spawn_local(async move {
                log("  📻 Starting notification listener...");
                let mut notifications = client.notifications();

                while let Ok(notification) = notifications.recv().await {
                    if let RelayPoolNotification::Event { event, .. } = notification {
                        log(&format!("  📩 Received event: {}", event.id.to_hex()));

                        // Create MDK instance and process the message
                        match create_mdk().await {
                            Ok(mdk) => {
                                match mdk.process_message(&event) {
                                    Ok(result) => {
                                        use mdk_core::prelude::MessageProcessingResult;
                                        if let MessageProcessingResult::ApplicationMessage(msg) = result {
                                            log(&format!("  ✅ Application message: '{}'", msg.content));
                                            log(&format!("     Message group ID: {}", hex::encode(msg.mls_group_id.as_slice())));
                                            log(&format!("     Target group ID: {}", hex::encode(group_id.as_slice())));

                                            // Check if this message belongs to the current group
                                            if msg.mls_group_id == group_id {
                                                log("  🎯 Message matches current group! Calling callback...");

                                                // Prepare callback data
                                                let msg_data = MessageCallback {
                                                    id: msg.id.to_hex(),
                                                    pubkey: msg.pubkey.to_hex(),
                                                    content: msg.content,
                                                    created_at: msg.created_at.as_u64(),
                                                    state: msg.state.to_string(),
                                                };

                                                // Call the JavaScript callback
                                                if let Ok(js_value) = serde_wasm_bindgen::to_value(&msg_data) {
                                                    match callback.call1(&JsValue::NULL, &js_value) {
                                                        Ok(_) => log("  ✅ Callback invoked successfully"),
                                                        Err(e) => log(&format!("  ❌ Callback failed: {:?}", e)),
                                                    }
                                                } else {
                                                    log("  ❌ Failed to serialize message to JS value");
                                                }
                                            } else {
                                                log("  ⏭️  Message is for a different group, skipping");
                                            }
                                        } else {
                                            log(&format!("  ℹ️  Non-application message: {:?}", result));
                                        }
                                    }
                                    Err(e) => {
                                        log(&format!("  ⚠️  Failed to process message: {}", e));
                                    }
                                }
                            }
                            Err(e) => {
                                log(&format!("  ⚠️  Failed to create MDK: {:?}", e));
                            }
                        }
                    }
                }
            });

            Ok::<(), JsValue>(())
        }
        .await;

        result.map(|_| JsValue::undefined())
    })
}

