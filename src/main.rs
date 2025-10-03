use anyhow::Result;
use eframe::egui;
use qrcode::QrCode;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

// MDK imports
use mdk_core::prelude::*;
use mdk_sqlite_storage::MdkSqliteStorage;
use nostr::event::builder::EventBuilder;
use nostr::{EventId, Keys, Kind, RelayUrl};
use nostr_sdk::{Client, Filter, RelayPoolNotification};
use std::fs;
use std::path::Path;

// CDK imports
use cdk::Amount;
use cdk::wallet::{Wallet, WalletBuilder, ReceiveOptions, SendOptions};
use cdk::nuts::{CurrencyUnit, Token};
use cdk::mint_url::MintUrl;
use cdk_sqlite::WalletSqliteDatabase;

#[derive(Clone)]
struct User {
    name: String,
    keys: Keys,
    mdk: Arc<Mutex<MDK<MdkSqliteStorage>>>,
    wallet: Wallet,
    mls_group_id: Option<GroupId>,
    nostr_client: Client,
}

#[derive(Clone)]
struct Message {
    sender: String,
    content: String,
    timestamp: u64, // Unix epoch seconds
}

#[derive(Clone)]
struct AppState {
    users: Vec<User>,
    messages: Arc<Mutex<Vec<Message>>>,
    relay_urls: Vec<RelayUrl>,
    pending_qr: Arc<Mutex<Option<(String, String, u64)>>>, // (user_name, invoice, amount)
    balances: Arc<Mutex<Vec<u64>>>, // Cached balances for each user
}

// Helper functions for key persistence
fn save_keys(name: &str, keys: &Keys) -> Result<()> {
    let keys_dir = Path::new("./keys");
    fs::create_dir_all(keys_dir)?;
    let key_file = keys_dir.join(format!("{}.key", name));
    let secret_key_hex = keys.secret_key().to_secret_hex();
    fs::write(key_file, secret_key_hex)?;
    Ok(())
}

fn load_or_create_keys(name: &str) -> Result<Keys> {
    let keys_dir = Path::new("./keys");
    let key_file = keys_dir.join(format!("{}.key", name));

    if key_file.exists() {
        let secret_key_hex = fs::read_to_string(key_file)?;
        let keys = Keys::parse(&secret_key_hex)?;
        tracing::info!("{} loaded existing keys: {}", name, keys.public_key());
        Ok(keys)
    } else {
        let keys = Keys::generate();
        save_keys(name, &keys)?;
        tracing::info!("{} generated new keys: {}", name, keys.public_key());
        Ok(keys)
    }
}

impl AppState {
    async fn new() -> Result<Self> {
        // Use local relay
        let relay_urls = vec![
            RelayUrl::parse("ws://localhost:8080")?,
        ];

        tracing::info!("Connecting to relay: ws://localhost:8080");

        // Test mint URL (testnut - no real sats)
        let mint_url = MintUrl::from_str("https://nofees.testnut.cashu.space")?;
        tracing::info!("Using mint: {}", mint_url);

        // Create three users with persistent keys and storage
        let alice_keys = load_or_create_keys("alice")?;
        let alice_storage = MdkSqliteStorage::new("./mdk_storage/alice.db")?;
        let alice_mdk = Arc::new(Mutex::new(MDK::new(alice_storage)));
        let alice_client = Client::new(alice_keys.clone());

        // Create Alice's wallet with SQLite storage
        let alice_db = WalletSqliteDatabase::new("./wallets/alice.db").await?;
        let mut alice_seed = [0u8; 64];
        alice_seed[..32].copy_from_slice(alice_keys.secret_key().as_secret_bytes());
        let alice_wallet = WalletBuilder::new()
            .mint_url(mint_url.clone())
            .unit(CurrencyUnit::Sat)
            .localstore(Arc::new(alice_db))
            .seed(alice_seed)
            .build()?;

        let bob_keys = load_or_create_keys("bob")?;
        let bob_storage = MdkSqliteStorage::new("./mdk_storage/bob.db")?;
        let bob_mdk = Arc::new(Mutex::new(MDK::new(bob_storage)));
        let bob_client = Client::new(bob_keys.clone());

        // Create Bob's wallet with SQLite storage
        let bob_db = WalletSqliteDatabase::new("./wallets/bob.db").await?;
        let mut bob_seed = [0u8; 64];
        bob_seed[..32].copy_from_slice(bob_keys.secret_key().as_secret_bytes());
        let bob_wallet = WalletBuilder::new()
            .mint_url(mint_url.clone())
            .unit(CurrencyUnit::Sat)
            .localstore(Arc::new(bob_db))
            .seed(bob_seed)
            .build()?;

        let carol_keys = load_or_create_keys("carol")?;
        let carol_storage = MdkSqliteStorage::new("./mdk_storage/carol.db")?;
        let carol_mdk = Arc::new(Mutex::new(MDK::new(carol_storage)));
        let carol_client = Client::new(carol_keys.clone());

        // Create Carol's wallet with SQLite storage
        let carol_db = WalletSqliteDatabase::new("./wallets/carol.db").await?;
        let mut carol_seed = [0u8; 64];
        carol_seed[..32].copy_from_slice(carol_keys.secret_key().as_secret_bytes());
        let carol_wallet = WalletBuilder::new()
            .mint_url(mint_url.clone())
            .unit(CurrencyUnit::Sat)
            .localstore(Arc::new(carol_db))
            .seed(carol_seed)
            .build()?;

        // Add relays to clients (for when real relays are enabled)
        for relay_url in &relay_urls {
            alice_client.add_relay(relay_url.as_str()).await?;
            bob_client.add_relay(relay_url.as_str()).await?;
            carol_client.add_relay(relay_url.as_str()).await?;
        }

        // Check if groups already exist (for restarts)
        let alice_groups = alice_mdk.lock().unwrap().get_groups()?;
        let bob_groups = bob_mdk.lock().unwrap().get_groups()?;
        let carol_groups = carol_mdk.lock().unwrap().get_groups()?;

        let (alice_group_id, bob_group_id, carol_group_id) = if !alice_groups.is_empty() && !bob_groups.is_empty() && !carol_groups.is_empty() {
            // Groups exist, load them
            let alice_group_id = alice_groups[0].mls_group_id.clone();
            let bob_group_id = bob_groups[0].mls_group_id.clone();
            let carol_group_id = carol_groups[0].mls_group_id.clone();

            tracing::info!("Loaded existing group with ID: {}", hex::encode(alice_group_id.as_slice()));
            (alice_group_id, bob_group_id, carol_group_id)
        } else {
            // Create new group
            tracing::info!("No existing group found, creating new one...");

            // Create key packages for Bob and Carol
            let (bob_key_package, bob_tags) = bob_mdk
                .lock()
                .unwrap()
                .create_key_package_for_event(&bob_keys.public_key(), relay_urls.clone())?;

            let bob_key_package_event = EventBuilder::new(Kind::MlsKeyPackage, bob_key_package)
                .tags(bob_tags)
                .build(bob_keys.public_key())
                .sign(&bob_keys)
                .await?;

            let (carol_key_package, carol_tags) = carol_mdk
                .lock()
                .unwrap()
                .create_key_package_for_event(&carol_keys.public_key(), relay_urls.clone())?;

            let carol_key_package_event = EventBuilder::new(Kind::MlsKeyPackage, carol_key_package)
                .tags(carol_tags)
                .build(carol_keys.public_key())
                .sign(&carol_keys)
                .await?;

            // Alice creates a group with Bob and Carol
            let config = NostrGroupConfigData::new(
                "Group Chat".to_string(),
                "Demo group chat".to_string(),
                None,
                None,
                None,
                relay_urls.clone(),
                vec![
                    alice_keys.public_key(),
                    bob_keys.public_key(),
                    carol_keys.public_key(),
                ],
            );

            let group_result = alice_mdk.lock().unwrap().create_group(
                &alice_keys.public_key(),
                vec![bob_key_package_event, carol_key_package_event],
                config,
            )?;

            let alice_group_id = group_result.group.mls_group_id.clone();

            // Log the group ID for debugging
            let group_id_hex = hex::encode(alice_group_id.as_slice());
            tracing::info!("MLS Group created with ID: {}", group_id_hex);

            // Bob processes welcome and joins
            let bob_welcome_rumor = &group_result.welcome_rumors[0];
            bob_mdk
                .lock()
                .unwrap()
                .process_welcome(&EventId::all_zeros(), bob_welcome_rumor)?;
            let bob_welcomes = bob_mdk.lock().unwrap().get_pending_welcomes()?;
            bob_mdk.lock().unwrap().accept_welcome(&bob_welcomes[0])?;
            let bob_group_id = bob_mdk.lock().unwrap().get_groups()?[0]
                .mls_group_id
                .clone();

            // Carol processes welcome and joins
            let carol_welcome_rumor = &group_result.welcome_rumors[1];
            carol_mdk
                .lock()
                .unwrap()
                .process_welcome(&EventId::all_zeros(), carol_welcome_rumor)?;
            let carol_welcomes = carol_mdk.lock().unwrap().get_pending_welcomes()?;
            carol_mdk
                .lock()
                .unwrap()
                .accept_welcome(&carol_welcomes[0])?;
            let carol_group_id = carol_mdk.lock().unwrap().get_groups()?[0]
                .mls_group_id
                .clone();

            (alice_group_id, bob_group_id, carol_group_id)
        };

        let users = vec![
            User {
                name: "Alice".to_string(),
                keys: alice_keys,
                mdk: alice_mdk.clone(),
                wallet: alice_wallet,
                mls_group_id: Some(alice_group_id.clone()),
                nostr_client: alice_client,
            },
            User {
                name: "Bob".to_string(),
                keys: bob_keys,
                mdk: bob_mdk.clone(),
                wallet: bob_wallet,
                mls_group_id: Some(bob_group_id.clone()),
                nostr_client: bob_client,
            },
            User {
                name: "Carol".to_string(),
                keys: carol_keys,
                mdk: carol_mdk.clone(),
                wallet: carol_wallet,
                mls_group_id: Some(carol_group_id.clone()),
                nostr_client: carol_client,
            },
        ];

        // Load historical messages from MDK storage (already decrypted)
        let mut historical_messages = Vec::new();
        if let Ok(mut msgs) = alice_mdk.lock().unwrap().get_messages(&alice_group_id) {
            // Sort messages by created_at timestamp (oldest first)
            msgs.sort_by_key(|m| m.created_at);

            tracing::info!("Loading {} historical messages from MDK storage:", msgs.len());
            for (i, msg) in msgs.iter().enumerate() {
                // Parse tab-delimited format: timestamp\tusername\tcontent
                let parts: Vec<&str> = msg.content.splitn(3, '\t').collect();

                let (timestamp, sender_name, content) = if parts.len() == 3 {
                    // Parse timestamp, username, and content from message
                    let ts = parts[0].parse::<u64>().unwrap_or(msg.created_at.as_u64());
                    let username = parts[1].to_string();
                    let content = parts[2].to_string();
                    (ts, username, content)
                } else {
                    // Fallback for messages without tab-delimited format
                    let sender = format!("User-{}", &msg.pubkey.to_string()[..8]);
                    (msg.created_at.as_u64(), sender, msg.content.clone())
                };

                tracing::info!("  [{}] {} at {}: {}", i, sender_name, timestamp, content);
                historical_messages.push(Message {
                    sender: sender_name,
                    content,
                    timestamp,
                });
            }
            tracing::info!("Loaded {} historical messages in chronological order", historical_messages.len());
        }

        // Fetch initial balances from wallets
        let mut initial_balances = vec![0u64; 3];
        for (i, user) in users.iter().enumerate() {
            match user.wallet.total_balance().await {
                Ok(balance) => {
                    initial_balances[i] = balance.into();
                    tracing::info!("{} initial balance: {} sats", user.name, balance);
                }
                Err(e) => {
                    tracing::warn!("{} failed to fetch initial balance: {}", user.name, e);
                }
            }
        }

        let state = Self {
            users,
            messages: Arc::new(Mutex::new(historical_messages)),
            relay_urls,
            pending_qr: Arc::new(Mutex::new(None)),
            balances: Arc::new(Mutex::new(initial_balances)),
        };

        // Connect to relays and start listening
        for user in &state.users {
            user.nostr_client.connect().await;
        }
        tracing::info!("Connected to Nostr relays");

        // Start background tasks to listen for messages
        state.start_relay_listeners().await?;

        Ok(state)
    }

    async fn start_relay_listeners(&self) -> Result<()> {
        for (_user_index, user) in self.users.iter().enumerate() {
            let client = user.nostr_client.clone();
            let messages = self.messages.clone();
            let user_name = user.name.clone();
            let mdk = user.mdk.clone();
            let group_id = user.mls_group_id.clone().unwrap();

            // Convert group ID to hex string for filtering
            let _group_id_hex = hex::encode(group_id.as_slice());

            tokio::spawn(async move {
                tracing::info!("{} starting relay listener for group: {}", user_name, hex::encode(group_id.as_slice()));

                // Subscribe to recent events (10 seconds ago to now)
                // This ensures we catch any events that happen right after we connect
                let now = nostr::Timestamp::now();
                let recent = nostr::Timestamp::from(now.as_u64().saturating_sub(10));
                let filter = Filter::new()
                    .kind(nostr::Kind::MlsGroupMessage)
                    .since(recent);

                tracing::info!("{} subscribing to MLS messages (kind 445) since {} (10 sec buffer)", user_name, recent);

                match client.subscribe(filter.clone(), None).await {
                    Ok(sub_id) => {
                        tracing::info!("{} subscription successful! ID: {:?}", user_name, sub_id);
                    }
                    Err(e) => {
                        tracing::error!("{} subscription FAILED: {}", user_name, e);
                        return;
                    }
                }

                // Listen for notifications
                tracing::info!("{} starting notification loop...", user_name);
                let mut notifications = client.notifications();
                let mut event_count = 0;

                while let Ok(notification) = notifications.recv().await {
                    match &notification {
                        RelayPoolNotification::Event { relay_url, event, .. } => {
                            event_count += 1;
                            tracing::info!("{} received event #{} from {} (kind: {})", user_name, event_count, relay_url, event.kind);

                            // Try to process the message through MDK
                            let process_result = {
                                let mdk_guard = mdk.lock().unwrap();
                                mdk_guard.process_message(event)
                            };

                            match process_result {
                                Ok(result) => {
                                    tracing::info!("{} MDK processed event successfully (event_id: {})", user_name, event.id);

                                    // Check if this is an application message (chat message)
                                    if let mdk_core::prelude::MessageProcessingResult::ApplicationMessage(msg) = result {
                                        // Parse tab-delimited format: timestamp\tusername\tcontent
                                        let parts: Vec<&str> = msg.content.splitn(3, '\t').collect();

                                        let (timestamp, sender_name, content) = if parts.len() == 3 {
                                            // Parse timestamp, username, and content from message
                                            let ts = parts[0].parse::<u64>().unwrap_or(msg.created_at.as_u64());
                                            let username = parts[1].to_string();
                                            let content = parts[2].to_string();
                                            (ts, username, content)
                                        } else {
                                            // Fallback for messages without tab-delimited format
                                            let sender = format!("User-{}", &msg.pubkey.to_string()[..8]);
                                            (msg.created_at.as_u64(), sender, msg.content.clone())
                                        };

                                        tracing::info!("{} received APPLICATION MESSAGE: '{}' from {} at {}",
                                            user_name, content, sender_name, timestamp);

                                        // Check if this message is already in the shared list (by timestamp + content)
                                        let already_exists = {
                                            let messages_guard = messages.lock().unwrap();
                                            let exists = messages_guard.iter().any(|m| {
                                                m.timestamp == timestamp && m.content == content
                                            });
                                            tracing::info!("{} checking if message exists in GUI: {}", user_name, exists);
                                            exists
                                        };

                                        if !already_exists {
                                            messages.lock().unwrap().push(Message {
                                                sender: sender_name.clone(),
                                                content: content.clone(),
                                                timestamp,
                                            });
                                            tracing::info!("{} added NEW message to GUI: {} says '{}'", user_name, sender_name, content);
                                        } else {
                                            tracing::info!("{} message already exists in GUI, skipping", user_name);
                                        }
                                    } else {
                                        tracing::debug!("{} processed non-application message: {:?}", user_name, result);
                                    }
                                }
                                Err(e) => {
                                    tracing::debug!("{} couldn't process event ({}): {}", user_name, event.id, e);
                                }
                            }
                        }
                        other => {
                            tracing::debug!("{} received other notification: {:?}", user_name, other);
                        }
                    }
                }

                tracing::warn!("{} notification loop ended!", user_name);
            });
        }
        Ok(())
    }

    async fn send_message(&self, user_index: usize, content: String) -> Result<()> {
        let user = &self.users[user_index];
        let group_id = user.mls_group_id.as_ref().unwrap();

        // Prepend timestamp (Unix epoch seconds) and username to message content
        let now = nostr::Timestamp::now();
        let content_with_metadata = format!("{}\t{}\t{}", now.as_u64(), user.name, content);

        // Create message
        let rumor = EventBuilder::new(Kind::Custom(9), &content_with_metadata).build(user.keys.public_key());

        let message_event = user
            .mdk
            .lock()
            .unwrap()
            .create_message(group_id, rumor)?;

        // Log event details before publishing
        tracing::info!("{} sending message event:", user.name);
        tracing::info!("  Event ID: {}", message_event.id);
        tracing::info!("  Kind: {}", message_event.kind);
        tracing::info!("  ALL Tags: {:?}", message_event.tags);

        // Check for h tag specifically
        let h_tag = message_event.tags.iter().find(|t| {
            t.as_slice().first().map(|s| s.as_str()) == Some("h")
        });
        tracing::info!("  h tag found: {:?}", h_tag);

        // Publish to real Nostr relays
        let send_result = user.nostr_client.send_event(&message_event).await?;
        tracing::info!("{} published message to Nostr relays", user.name);

        // Log which relays accepted the event
        for relay_url in send_result.success.iter() {
            tracing::info!("  ✓ {} accepted the message", relay_url);
        }
        for relay_url in send_result.failed.keys() {
            if let Some(error) = send_result.failed.get(relay_url) {
                tracing::warn!("  ✗ {} rejected the message: {}", relay_url, error);
            }
        }

        // Don't add to message list - will come back via subscription

        Ok(())
    }
}

struct ChatApp {
    state: AppState,
    input_texts: [String; 3],
    zoom_level: f32,
}

impl ChatApp {
    fn new(state: AppState) -> Self {
        Self {
            state,
            input_texts: [String::new(), String::new(), String::new()],
            zoom_level: 2.4,
        }
    }

    fn format_message_content(content: &str) -> String {
        // Check if message contains a cashu token
        if let Some(token_str) = content.split_whitespace()
            .find(|word| word.starts_with("cashuA") || word.starts_with("cashuB")) {

            // Try to parse the token
            if let Ok(token) = Token::from_str(token_str) {
                // Get total value
                let total_value = token.value().unwrap_or(Amount::ZERO);

                // Get mint URL
                let mint_url = token.mint_url().ok();

                // Replace the token string with a nice summary
                let before = content.split(token_str).next().unwrap_or("");
                let after = content.split(token_str).nth(1).unwrap_or("");

                if let Some(url) = mint_url {
                    format!("{}[🎁 Cashu Token: {} sats from {}]{}",
                        before, total_value, url, after)
                } else {
                    format!("{}[🎁 Cashu Token: {} sats]{}",
                        before, total_value, after)
                }
            } else {
                content.to_string()
            }
        } else {
            content.to_string()
        }
    }

    fn render_user_pane(&mut self, ui: &mut egui::Ui, user_index: usize) {
        let user_name = self.state.users[user_index].name.clone();

        ui.vertical(|ui| {
            ui.heading(&user_name);
            ui.separator();

            // Wallet balance (cached)
            ui.horizontal(|ui| {
                ui.label("Balance:");
                let balance = self.state.balances.lock().unwrap()[user_index];
                ui.label(format!("{} sats", balance));
            });
            ui.separator();

            // Messages
            ui.label("Messages:");
            egui::ScrollArea::vertical()
                .id_salt(format!("messages_{}", user_index))
                .max_height(300.0)
                .auto_shrink([false; 2])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    let messages = self.state.messages.lock().unwrap();
                    for msg in messages.iter() {
                        let formatted_content = Self::format_message_content(&msg.content);

                        // Check if message contains a Cashu token
                        if formatted_content.contains("🎁 Cashu Token") {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(egui::RichText::new(format!("{}:", &msg.sender)).strong());
                                ui.label(
                                    egui::RichText::new(&formatted_content)
                                        .color(egui::Color32::from_rgb(255, 140, 0))
                                );
                            });
                        } else {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(egui::RichText::new(format!("{}:", &msg.sender)).strong());
                                ui.label(&formatted_content);
                            });
                        }
                    }
                });

            ui.separator();

            // Input area
            ui.label("Send message:");
            let response = ui.text_edit_singleline(&mut self.input_texts[user_index]);
            let should_send = (ui.button("Send").clicked() || response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                && !self.input_texts[user_index].is_empty();

            if should_send {
                let content = self.input_texts[user_index].clone();
                self.input_texts[user_index].clear();

                // Check for commands
                if content.starts_with("!") {
                    self.handle_command(user_index, &content);
                } else {
                    // Send regular message in background
                    let state = self.state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = state.send_message(user_index, content).await {
                            eprintln!("Error sending message: {}", e);
                        }
                    });
                }
            }
        });
    }

    fn handle_command(&mut self, user_index: usize, command: &str) {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return;
        }

        let wallet = self.state.users[user_index].wallet.clone();
        let user_name = self.state.users[user_index].name.clone();

        match parts[0] {
            "!topup" => {
                // Parse amount (default to 100 sats)
                let amount = if parts.len() > 1 {
                    parts[1].parse::<u64>().unwrap_or(100)
                } else {
                    100
                };

                // Create mint quote in background
                let user_name_clone = user_name.clone();
                let pending_qr = self.state.pending_qr.clone();

                tokio::spawn(async move {
                    match wallet.mint_quote(Amount::from(amount), None).await {
                        Ok(quote) => {
                            tracing::info!("{} created mint quote: request={}, id={}",
                                user_name_clone, quote.request, quote.id);
                            println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                            println!("Lightning Invoice for {} ({} sats):", user_name_clone, amount);
                            println!("{}", quote.request);
                            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

                            // Set the QR popup data
                            *pending_qr.lock().unwrap() = Some((user_name_clone.clone(), quote.request, amount));
                        }
                        Err(e) => {
                            tracing::error!("{} failed to create mint quote: {}", user_name_clone, e);
                        }
                    }
                });

                tracing::info!("{} requested topup of {} sats", user_name, amount);
            }
            "!redeem" => {
                if parts.len() < 2 {
                    tracing::warn!("{} !redeem requires a token", user_name);
                    self.state.messages.lock().unwrap().push(Message {
                        sender: "SYSTEM".to_string(),
                        content: format!("{}: !redeem requires a token", user_name),
                        timestamp: nostr::Timestamp::now().as_u64(),
                    });
                    return;
                }

                let token = parts[1..].join(" ");
                let user_name_clone = user_name.clone();
                let wallet_clone = wallet.clone();
                let messages = self.state.messages.clone();
                let balances = self.state.balances.clone();

                // Add initial feedback
                messages.lock().unwrap().push(Message {
                    sender: "SYSTEM".to_string(),
                    content: format!("{}: Redeeming token...", user_name),
                    timestamp: nostr::Timestamp::now().as_u64(),
                });

                tokio::spawn(async move {
                    match wallet_clone.receive(&token, ReceiveOptions::default()).await {
                        Ok(amount) => {
                            tracing::info!("{} successfully redeemed {} sats!", user_name_clone, amount);
                            println!("\n✅ {} received {} sats\n", user_name_clone, amount);

                            // Fetch updated balance
                            match wallet_clone.total_balance().await {
                                Ok(new_balance) => {
                                    balances.lock().unwrap()[user_index] = new_balance.into();
                                    messages.lock().unwrap().push(Message {
                                        sender: "SYSTEM".to_string(),
                                        content: format!("{}: ✅ Received {} sats! New balance: {} sats",
                                            user_name_clone, amount, new_balance),
                                        timestamp: nostr::Timestamp::now().as_u64(),
                                    });
                                }
                                Err(e) => {
                                    tracing::error!("{} failed to fetch balance: {}", user_name_clone, e);
                                    messages.lock().unwrap().push(Message {
                                        sender: "SYSTEM".to_string(),
                                        content: format!("{}: ✅ Received {} sats (balance fetch failed)",
                                            user_name_clone, amount),
                                        timestamp: nostr::Timestamp::now().as_u64(),
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("{} failed to redeem token: {}", user_name_clone, e);
                            messages.lock().unwrap().push(Message {
                                sender: "SYSTEM".to_string(),
                                content: format!("{}: ❌ Failed to redeem: {}", user_name_clone, e),
                                timestamp: nostr::Timestamp::now().as_u64(),
                            });
                        }
                    }
                });

                tracing::info!("{} attempting to redeem token", user_name);
            }
            "!redeemlast" => {
                // Find the most recent message with a cashu token
                let messages_lock = self.state.messages.lock().unwrap();
                let token_opt = messages_lock.iter().rev().find_map(|msg| {
                    // Look for cashuA or cashuB tokens in the message
                    msg.content.split_whitespace()
                        .find(|word| word.starts_with("cashuA") || word.starts_with("cashuB"))
                        .map(|s| s.to_string())
                });
                drop(messages_lock);

                if let Some(token) = token_opt {
                    let user_name_clone = user_name.clone();
                    let wallet_clone = wallet.clone();
                    let messages = self.state.messages.clone();
                    let balances = self.state.balances.clone();

                    // Add initial feedback
                    messages.lock().unwrap().push(Message {
                        sender: "SYSTEM".to_string(),
                        content: format!("{}: Redeeming last token...", user_name),
                        timestamp: nostr::Timestamp::now().as_u64(),
                    });

                    tokio::spawn(async move {
                        match wallet_clone.receive(&token, ReceiveOptions::default()).await {
                            Ok(amount) => {
                                tracing::info!("{} successfully redeemed {} sats!", user_name_clone, amount);
                                println!("\n✅ {} received {} sats\n", user_name_clone, amount);

                                // Fetch updated balance
                                match wallet_clone.total_balance().await {
                                    Ok(new_balance) => {
                                        balances.lock().unwrap()[user_index] = new_balance.into();
                                        messages.lock().unwrap().push(Message {
                                            sender: "SYSTEM".to_string(),
                                            content: format!("{}: ✅ Received {} sats! New balance: {} sats",
                                                user_name_clone, amount, new_balance),
                                            timestamp: nostr::Timestamp::now().as_u64(),
                                        });
                                    }
                                    Err(e) => {
                                        tracing::error!("{} failed to fetch balance: {}", user_name_clone, e);
                                        messages.lock().unwrap().push(Message {
                                            sender: "SYSTEM".to_string(),
                                            content: format!("{}: ✅ Received {} sats (balance fetch failed)",
                                                user_name_clone, amount),
                                            timestamp: nostr::Timestamp::now().as_u64(),
                                        });
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("{} failed to redeem token: {}", user_name_clone, e);
                                messages.lock().unwrap().push(Message {
                                    sender: "SYSTEM".to_string(),
                                    content: format!("{}: ❌ Failed to redeem: {}", user_name_clone, e),
                                    timestamp: nostr::Timestamp::now().as_u64(),
                                });
                            }
                        }
                    });

                    tracing::info!("{} attempting to redeem last token", user_name);
                } else {
                    self.state.messages.lock().unwrap().push(Message {
                        sender: "SYSTEM".to_string(),
                        content: format!("{}: No cashu token found in recent messages", user_name),
                        timestamp: nostr::Timestamp::now().as_u64(),
                    });
                    tracing::warn!("{} no cashu token found in messages", user_name);
                }
            }
            "!send" => {
                // Parse amount (default to 10 sats)
                let amount = if parts.len() > 1 {
                    parts[1].parse::<u64>().unwrap_or(10)
                } else {
                    10
                };

                let user_name_clone = user_name.clone();
                let wallet_clone = wallet.clone();
                let messages = self.state.messages.clone();
                let balances = self.state.balances.clone();
                let state = self.state.clone();

                // Add initial feedback
                messages.lock().unwrap().push(Message {
                    sender: "SYSTEM".to_string(),
                    content: format!("{}: Creating {}-sat token...", user_name, amount),
                    timestamp: nostr::Timestamp::now().as_u64(),
                });

                tokio::spawn(async move {
                    // Prepare send
                    match wallet_clone.prepare_send(Amount::from(amount), SendOptions::default()).await {
                        Ok(prepared) => {
                            // Confirm send to get token
                            match prepared.confirm(None).await {
                                Ok(token) => {
                                    tracing::info!("{} created token for {} sats", user_name_clone, amount);

                                    // Fetch updated balance
                                    match wallet_clone.total_balance().await {
                                        Ok(new_balance) => {
                                            balances.lock().unwrap()[user_index] = new_balance.into();
                                        }
                                        Err(e) => {
                                            tracing::error!("{} failed to fetch balance: {}", user_name_clone, e);
                                        }
                                    }

                                    // Send token as MLS message to group (just the raw token string)
                                    let send_result = state.send_message(
                                        user_index,
                                        token.to_string()
                                    ).await;

                                    match send_result {
                                        Ok(_) => {
                                            messages.lock().unwrap().push(Message {
                                                sender: "SYSTEM".to_string(),
                                                content: format!("{}: ✅ Sent {}-sat token to group!", user_name_clone, amount),
                                                timestamp: nostr::Timestamp::now().as_u64(),
                                            });
                                        }
                                        Err(e) => {
                                            tracing::error!("{} failed to send message: {}", user_name_clone, e);
                                            messages.lock().unwrap().push(Message {
                                                sender: "SYSTEM".to_string(),
                                                content: format!("{}: ❌ Failed to broadcast token: {}", user_name_clone, e),
                                                timestamp: nostr::Timestamp::now().as_u64(),
                                            });
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("{} failed to confirm send: {}", user_name_clone, e);
                                    messages.lock().unwrap().push(Message {
                                        sender: "SYSTEM".to_string(),
                                        content: format!("{}: ❌ Failed to confirm send: {}", user_name_clone, e),
                                        timestamp: nostr::Timestamp::now().as_u64(),
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("{} failed to prepare send: {}", user_name_clone, e);
                            messages.lock().unwrap().push(Message {
                                sender: "SYSTEM".to_string(),
                                content: format!("{}: ❌ Failed to create token: {}", user_name_clone, e),
                                timestamp: nostr::Timestamp::now().as_u64(),
                            });
                        }
                    }
                });

                tracing::info!("{} creating token for {} sats", user_name, amount);
            }
            _ => {
                tracing::warn!("{} unknown command: {}", user_name, parts[0]);
            }
        }
    }
}

impl eframe::App for ChatApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply zoom
        ctx.set_pixels_per_point(self.zoom_level);

        egui::TopBottomPanel::top("zoom_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Zoom:");
                if ui.button("➖").clicked() && self.zoom_level > 0.5 {
                    self.zoom_level -= 0.1;
                }
                ui.label(format!("{:.0}%", self.zoom_level * 100.0));
                if ui.button("➕").clicked() && self.zoom_level < 3.0 {
                    self.zoom_level += 0.1;
                }
                if ui.button("Reset").clicked() {
                    self.zoom_level = 1.0;
                }

                ui.separator();

                // Show relay connection
                ui.label("Connected to relay: ws://localhost:8080");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(3, |columns| {
                self.render_user_pane(&mut columns[0], 0); // Alice
                self.render_user_pane(&mut columns[1], 1); // Bob
                self.render_user_pane(&mut columns[2], 2); // Carol
            });
        });

        // Show QR code popup if available
        let mut close_popup = false;
        if let Some((user_name, invoice, amount)) = self.state.pending_qr.lock().unwrap().clone() {
            egui::Window::new(format!("⚡ Lightning Invoice - {} ({} sats)", user_name, amount))
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        // Generate QR code
                        if let Ok(code) = QrCode::new(&invoice) {
                            let qr_size = 400;
                            let module_size = qr_size / code.width();

                            // Create image data
                            let qr_image = code.render::<image::Luma<u8>>()
                                .quiet_zone(false)
                                .min_dimensions(qr_size as u32, qr_size as u32)
                                .build();

                            // Convert to egui texture
                            let pixels: Vec<_> = qr_image.pixels()
                                .flat_map(|p| [p.0[0], p.0[0], p.0[0]])
                                .collect();

                            let color_image = egui::ColorImage::from_rgb(
                                [qr_image.width() as usize, qr_image.height() as usize],
                                &pixels,
                            );

                            let texture = ctx.load_texture(
                                "qr_code",
                                color_image,
                                egui::TextureOptions::LINEAR,
                            );

                            ui.image(&texture);
                        } else {
                            ui.label("Failed to generate QR code");
                        }

                        ui.add_space(10.0);
                        ui.label("Scan with Lightning wallet to pay");
                        ui.add_space(10.0);

                        // Invoice text (collapsible)
                        ui.collapsing("Show invoice text", |ui| {
                            ui.text_edit_multiline(&mut invoice.as_str());
                        });

                        ui.add_space(10.0);
                        if ui.button("Close").clicked() {
                            close_popup = true;
                        }
                    });
                });
        }

        if close_popup {
            *self.state.pending_qr.lock().unwrap() = None;
        }

        // Request repaint to update messages
        ctx.request_repaint();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // Initialize app state
    println!("Initializing group chat...");
    let state = AppState::new().await?;
    println!("Group chat initialized!");

    // Create single window with three panes
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "MDK Group Chat",
        options,
        Box::new(|_cc| Ok(Box::new(ChatApp::new(state)))),
    )
    .map_err(|e| anyhow::anyhow!("Failed to run window: {}", e))?;

    Ok(())
}
