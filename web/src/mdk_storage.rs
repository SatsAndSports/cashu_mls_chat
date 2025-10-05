use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;
use web_sys::window;

use mdk_storage_traits::GroupId;
use mdk_storage_traits::groups::{GroupStorage, types::{Group, GroupExporterSecret, GroupRelay}, error::GroupError};
use mdk_storage_traits::messages::{MessageStorage, types::{Message, ProcessedMessage}, error::MessageError};
use mdk_storage_traits::welcomes::{WelcomeStorage, types::{Welcome, ProcessedWelcome}, error::WelcomeError};
use mdk_storage_traits::{Backend, MdkStorageProvider};
use nostr::{EventId, PublicKey, RelayUrl};
use openmls_memory_storage::MemoryStorage;

// Helper types for serialization since some types don't implement Serialize/Deserialize directly
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializableState {
    groups: HashMap<String, Group>,  // GroupId as hex string
    groups_by_nostr_id: HashMap<String, Group>,  // [u8; 32] as hex string
    group_relays: HashMap<String, BTreeSet<GroupRelay>>,  // GroupId as hex string
    welcomes: HashMap<String, Welcome>,  // EventId as hex string
    processed_welcomes: HashMap<String, ProcessedWelcome>,  // EventId as hex string
    messages: HashMap<String, Message>,  // EventId as hex string
    messages_by_group: HashMap<String, Vec<Message>>,  // GroupId as hex string
    processed_messages: HashMap<String, ProcessedMessage>,  // EventId as hex string
    group_exporter_secrets: HashMap<String, GroupExporterSecret>,  // (GroupId, u64) as "hex:epoch"
}

#[derive(Debug, Clone, Default)]
struct MdkState {
    groups: HashMap<GroupId, Group>,
    groups_by_nostr_id: HashMap<[u8; 32], Group>,
    group_relays: HashMap<GroupId, BTreeSet<GroupRelay>>,
    welcomes: HashMap<EventId, Welcome>,
    processed_welcomes: HashMap<EventId, ProcessedWelcome>,
    messages: HashMap<EventId, Message>,
    messages_by_group: HashMap<GroupId, Vec<Message>>,
    processed_messages: HashMap<EventId, ProcessedMessage>,
    group_exporter_secrets: HashMap<(GroupId, u64), GroupExporterSecret>,
}

impl MdkState {
    fn to_serializable(&self) -> SerializableState {
        SerializableState {
            groups: self.groups.iter()
                .map(|(k, v)| (hex::encode(k.as_slice()), v.clone()))
                .collect(),
            groups_by_nostr_id: self.groups_by_nostr_id.iter()
                .map(|(k, v)| (hex::encode(k), v.clone()))
                .collect(),
            group_relays: self.group_relays.iter()
                .map(|(k, v)| (hex::encode(k.as_slice()), v.clone()))
                .collect(),
            welcomes: self.welcomes.iter()
                .map(|(k, v)| (k.to_hex(), v.clone()))
                .collect(),
            processed_welcomes: self.processed_welcomes.iter()
                .map(|(k, v)| (k.to_hex(), v.clone()))
                .collect(),
            messages: self.messages.iter()
                .map(|(k, v)| (k.to_hex(), v.clone()))
                .collect(),
            messages_by_group: self.messages_by_group.iter()
                .map(|(k, v)| (hex::encode(k.as_slice()), v.clone()))
                .collect(),
            processed_messages: self.processed_messages.iter()
                .map(|(k, v)| (k.to_hex(), v.clone()))
                .collect(),
            group_exporter_secrets: self.group_exporter_secrets.iter()
                .map(|((gid, epoch), v)| (format!("{}:{}", hex::encode(gid.as_slice()), epoch), v.clone()))
                .collect(),
        }
    }

    fn from_serializable(s: SerializableState) -> Result<Self, String> {
        Ok(Self {
            groups: s.groups.into_iter()
                .map(|(k, v)| {
                    let bytes = hex::decode(&k).map_err(|e| e.to_string())?;
                    Ok((GroupId::from_slice(&bytes), v))
                })
                .collect::<Result<_, String>>()?,
            groups_by_nostr_id: s.groups_by_nostr_id.into_iter()
                .map(|(k, v)| {
                    let bytes = hex::decode(&k).map_err(|e| e.to_string())?;
                    let arr: [u8; 32] = bytes.try_into().map_err(|_| "Invalid nostr_group_id length")?;
                    Ok((arr, v))
                })
                .collect::<Result<_, String>>()?,
            group_relays: s.group_relays.into_iter()
                .map(|(k, v)| {
                    let bytes = hex::decode(&k).map_err(|e| e.to_string())?;
                    Ok((GroupId::from_slice(&bytes), v))
                })
                .collect::<Result<_, String>>()?,
            welcomes: s.welcomes.into_iter()
                .map(|(k, v)| {
                    let event_id = EventId::from_hex(&k).map_err(|e| e.to_string())?;
                    Ok((event_id, v))
                })
                .collect::<Result<_, String>>()?,
            processed_welcomes: s.processed_welcomes.into_iter()
                .map(|(k, v)| {
                    let event_id = EventId::from_hex(&k).map_err(|e| e.to_string())?;
                    Ok((event_id, v))
                })
                .collect::<Result<_, String>>()?,
            messages: s.messages.into_iter()
                .map(|(k, v)| {
                    let event_id = EventId::from_hex(&k).map_err(|e| e.to_string())?;
                    Ok((event_id, v))
                })
                .collect::<Result<_, String>>()?,
            messages_by_group: s.messages_by_group.into_iter()
                .map(|(k, v)| {
                    let bytes = hex::decode(&k).map_err(|e| e.to_string())?;
                    Ok((GroupId::from_slice(&bytes), v))
                })
                .collect::<Result<_, String>>()?,
            processed_messages: s.processed_messages.into_iter()
                .map(|(k, v)| {
                    let event_id = EventId::from_hex(&k).map_err(|e| e.to_string())?;
                    Ok((event_id, v))
                })
                .collect::<Result<_, String>>()?,
            group_exporter_secrets: s.group_exporter_secrets.into_iter()
                .map(|(k, v)| {
                    let parts: Vec<&str> = k.split(':').collect();
                    if parts.len() != 2 {
                        return Err("Invalid group_exporter_secret key format".to_string());
                    }
                    let gid_bytes = hex::decode(parts[0]).map_err(|e| e.to_string())?;
                    let gid = GroupId::from_slice(&gid_bytes);
                    let epoch: u64 = parts[1].parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
                    Ok(((gid, epoch), v))
                })
                .collect::<Result<_, String>>()?,
        })
    }
}

#[derive(Debug)]
pub struct MdkHybridStorage {
    state: Arc<Mutex<MdkState>>,
    openmls_storage: MemoryStorage,
}

impl MdkHybridStorage {
    /// Load MDK state from localStorage
    fn load_mdk_state() -> Result<MdkState, JsValue> {
        let storage = window()
            .ok_or_else(|| JsValue::from_str("No window"))?
            .local_storage()?
            .ok_or_else(|| JsValue::from_str("No localStorage"))?;

        let json = storage
            .get_item("mdk_state")?
            .ok_or_else(|| JsValue::from_str("No MDK state found"))?;

        let serializable: SerializableState = serde_json::from_str(&json)
            .map_err(|e| JsValue::from_str(&format!("Deserialization error: {}", e)))?;

        let state = MdkState::from_serializable(serializable)
            .map_err(|e| JsValue::from_str(&format!("State conversion error: {}", e)))?;

        Ok(state)
    }

    /// Save MDK state to localStorage
    fn save_mdk_state(&self) -> Result<(), JsValue> {
        let state = self.state.lock().unwrap();
        let serializable = state.to_serializable();
        let json = serde_json::to_string(&serializable)
            .map_err(|e| JsValue::from_str(&format!("Serialization error: {}", e)))?;

        let storage = window()
            .ok_or_else(|| JsValue::from_str("No window"))?
            .local_storage()?
            .ok_or_else(|| JsValue::from_str("No localStorage"))?;

        storage.set_item("mdk_state", &json)?;
        Ok(())
    }

    /// Load OpenMLS MemoryStorage from localStorage
    fn load_openmls_storage() -> Result<MemoryStorage, JsValue> {
        let storage = window()
            .ok_or_else(|| JsValue::from_str("No window"))?
            .local_storage()?
            .ok_or_else(|| JsValue::from_str("No localStorage"))?;

        let base64_data = match storage.get_item("openmls_storage")? {
            Some(data) => data,
            None => {
                log("No OpenMLS storage found, starting fresh");
                return Ok(MemoryStorage::default());
            }
        };

        // Decode from base64
        use base64::{Engine as _, engine::general_purpose};
        let bytes = general_purpose::STANDARD.decode(&base64_data)
            .map_err(|e| JsValue::from_str(&format!("Base64 decode error: {}", e)))?;

        // Deserialize the HashMap with hex string keys
        let string_map: std::collections::HashMap<String, String> = serde_json::from_slice(&bytes)
            .map_err(|e| JsValue::from_str(&format!("Failed to deserialize OpenMLS storage: {}", e)))?;

        // Convert hex string keys/values back to Vec<u8>
        let map: std::collections::HashMap<Vec<u8>, Vec<u8>> = string_map
            .into_iter()
            .map(|(k, v)| {
                let key = hex::decode(&k).map_err(|e| JsValue::from_str(&format!("Failed to decode key: {}", e)))?;
                let value = hex::decode(&v).map_err(|e| JsValue::from_str(&format!("Failed to decode value: {}", e)))?;
                Ok((key, value))
            })
            .collect::<Result<_, JsValue>>()?;

        // Create MemoryStorage from the HashMap
        let memory_storage = MemoryStorage {
            values: std::sync::RwLock::new(map),
        };

        log(&format!("Loaded OpenMLS storage with {} entries", memory_storage.values.read().unwrap().len()));

        Ok(memory_storage)
    }

    /// Save OpenMLS MemoryStorage to localStorage
    fn save_openmls_storage(&self) -> Result<(), JsValue> {
        // Get the values from MemoryStorage
        let values = self.openmls_storage.values.read().unwrap();

        log(&format!("Saving OpenMLS storage with {} entries", values.len()));

        // Convert Vec<u8> keys/values to hex strings for JSON compatibility
        let string_map: std::collections::HashMap<String, String> = values
            .iter()
            .map(|(k, v)| (hex::encode(k), hex::encode(v)))
            .collect();

        // Serialize the HashMap with string keys to JSON
        let bytes = serde_json::to_vec(&string_map)
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize OpenMLS storage: {}", e)))?;

        // Encode to base64 for localStorage
        use base64::{Engine as _, engine::general_purpose};
        let base64_data = general_purpose::STANDARD.encode(&bytes);

        let storage = window()
            .ok_or_else(|| JsValue::from_str("No window"))?
            .local_storage()?
            .ok_or_else(|| JsValue::from_str("No localStorage"))?;

        storage.set_item("openmls_storage", &base64_data)?;

        Ok(())
    }

    pub async fn new() -> Result<Self, JsValue> {
        // Load MDK state (group metadata, messages, etc.)
        let state = match Self::load_mdk_state() {
            Ok(state) => {
                log("Loaded MDK state from localStorage");
                state
            }
            Err(_) => {
                log("No existing MDK state, starting fresh");
                MdkState::default()
            }
        };

        // Load OpenMLS storage (MLS encryption state)
        let openmls_storage = Self::load_openmls_storage()?;

        let storage = Self {
            state: Arc::new(Mutex::new(state)),
            openmls_storage,
        };

        // Save immediately to ensure storage is initialized
        storage.save_snapshot()?;
        log("Initialized MDK storage");

        Ok(storage)
    }

    fn save_snapshot(&self) -> Result<(), JsValue> {
        // Save both MDK state and OpenMLS storage
        self.save_mdk_state()?;
        self.save_openmls_storage()?;
        Ok(())
    }
}

fn log(msg: &str) {
    web_sys::console::log_1(&JsValue::from_str(msg));
}

// Convert JsValue errors to domain-specific errors
fn to_group_error(e: JsValue) -> GroupError {
    GroupError::DatabaseError(format!("{:?}", e))
}

fn to_message_error(e: JsValue) -> MessageError {
    MessageError::DatabaseError(format!("{:?}", e))
}

fn to_welcome_error(e: JsValue) -> WelcomeError {
    WelcomeError::DatabaseError(format!("{:?}", e))
}

// Implement GroupStorage trait
impl GroupStorage for MdkHybridStorage {
    fn all_groups(&self) -> Result<Vec<Group>, GroupError> {
        Ok(self.state.lock().unwrap().groups.values().cloned().collect())
    }

    fn find_group_by_mls_group_id(&self, group_id: &GroupId) -> Result<Option<Group>, GroupError> {
        Ok(self.state.lock().unwrap().groups.get(group_id).cloned())
    }

    fn find_group_by_nostr_group_id(
        &self,
        nostr_group_id: &[u8; 32],
    ) -> Result<Option<Group>, GroupError> {
        Ok(self.state.lock().unwrap().groups_by_nostr_id.get(nostr_group_id).cloned())
    }

    fn save_group(&self, group: Group) -> Result<(), GroupError> {
        let mut state = self.state.lock().unwrap();
        state.groups_by_nostr_id.insert(group.nostr_group_id, group.clone());
        state.groups.insert(group.mls_group_id.clone(), group);
        drop(state);

        // Save to localStorage
        self.save_snapshot().map_err(to_group_error)
    }

    fn messages(&self, group_id: &GroupId) -> Result<Vec<Message>, GroupError> {
        Ok(self.state.lock().unwrap()
            .messages_by_group
            .get(group_id)
            .cloned()
            .unwrap_or_default())
    }

    fn admins(&self, group_id: &GroupId) -> Result<BTreeSet<PublicKey>, GroupError> {
        Ok(self.state.lock().unwrap()
            .groups
            .get(group_id)
            .map(|g| g.admin_pubkeys.clone())
            .unwrap_or_default())
    }

    fn group_relays(&self, group_id: &GroupId) -> Result<BTreeSet<GroupRelay>, GroupError> {
        Ok(self.state.lock().unwrap()
            .group_relays
            .get(group_id)
            .cloned()
            .unwrap_or_default())
    }

    fn replace_group_relays(
        &self,
        group_id: &GroupId,
        relays: BTreeSet<RelayUrl>,
    ) -> Result<(), GroupError> {
        let mut state = self.state.lock().unwrap();
        let group_relays: BTreeSet<GroupRelay> = relays.into_iter()
            .map(|url| GroupRelay {
                relay_url: url,
                mls_group_id: group_id.clone(),
            })
            .collect();
        state.group_relays.insert(group_id.clone(), group_relays);
        drop(state);

        self.save_snapshot().map_err(to_group_error)
    }

    fn get_group_exporter_secret(
        &self,
        group_id: &GroupId,
        epoch: u64,
    ) -> Result<Option<GroupExporterSecret>, GroupError> {
        Ok(self.state.lock().unwrap()
            .group_exporter_secrets
            .get(&(group_id.clone(), epoch))
            .cloned())
    }

    fn save_group_exporter_secret(
        &self,
        group_exporter_secret: GroupExporterSecret,
    ) -> Result<(), GroupError> {
        let key = (group_exporter_secret.mls_group_id.clone(), group_exporter_secret.epoch);
        self.state.lock().unwrap()
            .group_exporter_secrets
            .insert(key, group_exporter_secret);

        self.save_snapshot().map_err(to_group_error)
    }
}

// Implement MessageStorage trait
impl MessageStorage for MdkHybridStorage {
    fn save_message(&self, message: Message) -> Result<(), MessageError> {
        let mut state = self.state.lock().unwrap();

        // Add to messages by event ID
        state.messages.insert(message.id, message.clone());

        // Add to messages by group
        state.messages_by_group
            .entry(message.mls_group_id.clone())
            .or_insert_with(Vec::new)
            .push(message);

        drop(state);

        self.save_snapshot().map_err(to_message_error)
    }

    fn find_message_by_event_id(&self, event_id: &EventId) -> Result<Option<Message>, MessageError> {
        Ok(self.state.lock().unwrap().messages.get(event_id).cloned())
    }

    fn save_processed_message(
        &self,
        processed_message: ProcessedMessage,
    ) -> Result<(), MessageError> {
        self.state.lock().unwrap()
            .processed_messages
            .insert(processed_message.wrapper_event_id, processed_message);

        self.save_snapshot().map_err(to_message_error)
    }

    fn find_processed_message_by_event_id(
        &self,
        event_id: &EventId,
    ) -> Result<Option<ProcessedMessage>, MessageError> {
        Ok(self.state.lock().unwrap()
            .processed_messages
            .get(event_id)
            .cloned())
    }
}

// Implement WelcomeStorage trait
impl WelcomeStorage for MdkHybridStorage {
    fn save_welcome(&self, welcome: Welcome) -> Result<(), WelcomeError> {
        self.state.lock().unwrap()
            .welcomes
            .insert(welcome.id, welcome);

        self.save_snapshot().map_err(to_welcome_error)
    }

    fn find_welcome_by_event_id(&self, event_id: &EventId) -> Result<Option<Welcome>, WelcomeError> {
        Ok(self.state.lock().unwrap().welcomes.get(event_id).cloned())
    }

    fn pending_welcomes(&self) -> Result<Vec<Welcome>, WelcomeError> {
        let state = self.state.lock().unwrap();
        let processed_ids: Vec<EventId> = state.processed_welcomes.keys().cloned().collect();

        Ok(state.welcomes.values()
            .filter(|w| !processed_ids.contains(&w.id))
            .cloned()
            .collect())
    }

    fn save_processed_welcome(
        &self,
        processed_welcome: ProcessedWelcome,
    ) -> Result<(), WelcomeError> {
        self.state.lock().unwrap()
            .processed_welcomes
            .insert(processed_welcome.wrapper_event_id, processed_welcome);

        self.save_snapshot().map_err(to_welcome_error)
    }

    fn find_processed_welcome_by_event_id(
        &self,
        event_id: &EventId,
    ) -> Result<Option<ProcessedWelcome>, WelcomeError> {
        Ok(self.state.lock().unwrap()
            .processed_welcomes
            .get(event_id)
            .cloned())
    }
}

// Implement MdkStorageProvider trait
impl MdkStorageProvider for MdkHybridStorage {
    type OpenMlsStorageProvider = MemoryStorage;

    fn backend(&self) -> Backend {
        Backend::Memory  // We report as Memory since we use MemoryStorage for OpenMLS
    }

    fn openmls_storage(&self) -> &Self::OpenMlsStorageProvider {
        &self.openmls_storage
    }

    fn openmls_storage_mut(&mut self) -> &mut Self::OpenMlsStorageProvider {
        &mut self.openmls_storage
    }
}
