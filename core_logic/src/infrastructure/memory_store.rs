use crate::domain::models::{
    AIError, ConversationTurn, MAX_CONVERSATION_CONTEXT_TURNS, MAX_CONVERSATION_MESSAGE_CHARS,
    MAX_CONVERSATION_TURNS, MAX_MEMORY_AGE_DAYS, MAX_MEMORY_FRAGMENTS, MemoryFragment,
    MemorySnapshot, MemorySyncOperation, MemorySyncPayload,
};
use crate::domain::traits::MemoryStore;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Serialize, Deserialize, Default)]
struct StorageData {
    revision: u64,
    last_synced_revision: u64,
    fragments: Vec<MemoryFragment>,
    user_profile: String,
    #[serde(default)]
    conversation_history: Vec<ConversationTurn>,
    #[serde(skip)]
    pending_sync: Vec<MemorySyncOperation>,
}

pub struct LocalMemoryStore {
    storage_path: PathBuf,
    data: Arc<RwLock<StorageData>>,
}

impl LocalMemoryStore {
    pub fn new(storage_dir: PathBuf) -> Self {
        let storage_path = storage_dir.join("memories.json");
        let data = if let Ok(content) = fs::read_to_string(&storage_path) {
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            StorageData::default()
        };

        Self {
            storage_path,
            data: Arc::new(RwLock::new(data)),
        }
    }

    async fn save_to_disk(&self) -> Result<(), AIError> {
        let data = self.data.read().await;
        let content = serde_json::to_string(&*data).map_err(|_| AIError::ParsingFailed)?;
        fs::write(&self.storage_path, content).map_err(|e| AIError::NetworkError(e.to_string()))?;
        Ok(())
    }

    async fn prune_memories(&self) {
        let mut data = self.data.write().await;
        let now = Utc::now();
        let cutoff = now - Duration::days(MAX_MEMORY_AGE_DAYS);
        let mut removed_ids = Vec::new();

        // 1) 时间过期淘汰
        data.fragments.retain(|f| {
            let keep = f.timestamp >= cutoff;
            if !keep {
                removed_ids.push(f.id);
            }
            keep
        });

        // 2) 超容量淘汰
        if data.fragments.len() > MAX_MEMORY_FRAGMENTS {
            data.fragments.sort_by(|a, b| {
                if a.importance != b.importance {
                    b.importance.cmp(&a.importance)
                } else {
                    b.timestamp.cmp(&a.timestamp)
                }
            });
            let truncated = data.fragments.split_off(MAX_MEMORY_FRAGMENTS);
            removed_ids.extend(truncated.into_iter().map(|fragment| fragment.id));
        }

        if !removed_ids.is_empty() {
            data.revision += 1;
            data.pending_sync.extend(
                removed_ids
                    .into_iter()
                    .map(|fragment_id| MemorySyncOperation::DeleteFragment { fragment_id }),
            );
        }
    }

    fn build_snapshot(data: &StorageData) -> MemorySnapshot {
        MemorySnapshot {
            revision: data.revision,
            fragments: data.fragments.clone(),
            user_profile: data.user_profile.clone(),
        }
    }

    fn normalize_content(raw: &str) -> String {
        let trimmed = raw.trim();
        if trimmed.chars().count() <= MAX_CONVERSATION_MESSAGE_CHARS {
            return trimmed.to_string();
        }
        let mut normalized = trimmed
            .chars()
            .take(MAX_CONVERSATION_MESSAGE_CHARS)
            .collect::<String>();
        normalized.push('…');
        normalized
    }

    fn normalize_role(raw: &str) -> String {
        match raw.trim().to_lowercase().as_str() {
            "assistant" => "assistant".to_string(),
            _ => "user".to_string(),
        }
    }
}

#[async_trait]
impl MemoryStore for LocalMemoryStore {
    async fn save_memory(&self, content: String, category: String) -> Result<(), AIError> {
        let mut data = self.data.write().await;
        // 去重
        if data.fragments.iter().any(|f| f.content == content) {
            return Ok(());
        }

        let fragment = MemoryFragment::new(content, category);
        data.fragments.push(fragment);
        let fragment = data
            .fragments
            .last()
            .cloned()
            .ok_or(AIError::ParsingFailed)?;
        data.revision += 1;
        data.pending_sync
            .push(MemorySyncOperation::UpsertFragment { fragment });

        drop(data); // 释放写锁以便执行 prune
        self.prune_memories().await;
        self.save_to_disk().await?;
        Ok(())
    }

    async fn get_all_fragments(&self) -> Result<Vec<MemoryFragment>, AIError> {
        let data = self.data.read().await;
        Ok(data.fragments.clone())
    }

    async fn save_user_profile(&self, profile: String) -> Result<(), AIError> {
        let mut data = self.data.write().await;
        data.user_profile = profile.trim().to_string();
        let profile = data.user_profile.clone();
        data.revision += 1;
        data.pending_sync
            .push(MemorySyncOperation::ReplaceUserProfile { profile });
        drop(data);
        self.save_to_disk().await?;
        Ok(())
    }

    async fn get_user_profile(&self) -> Result<String, AIError> {
        let data = self.data.read().await;
        Ok(data.user_profile.clone())
    }

    async fn get_long_term_context(&self) -> Result<String, AIError> {
        let data = self.data.read().await;
        let mut parts = Vec::new();

        // 对应 Swift 的 getLongTermContext 逻辑
        let profile = &data.user_profile;
        if !profile.is_empty() {
            parts.push(format!("【用户画像】\n{}", profile));
        } else {
            parts.push("【用户画像】\n(暂无)".to_string());
        }

        if !data.fragments.is_empty() {
            let bullets: Vec<String> = data
                .fragments
                .iter()
                .rev()
                .take(6)
                .map(|f| format!("- {}", f.content))
                .collect();
            parts.push(format!("【近期用户偏好/事实】\n{}", bullets.join("\n")));
        }

        let joined = parts.join("\n\n");
        // 限制总长度
        if joined.len() > 900 {
            Ok(joined[..900].to_string())
        } else {
            Ok(joined)
        }
    }

    async fn replace_snapshot(&self, snapshot: MemorySnapshot) -> Result<(), AIError> {
        let mut data = self.data.write().await;
        data.revision = snapshot.revision;
        data.last_synced_revision = snapshot.revision;
        data.fragments = snapshot.fragments;
        data.user_profile = snapshot.user_profile;
        data.pending_sync.clear();
        drop(data);
        self.save_to_disk().await?;
        Ok(())
    }

    async fn current_snapshot(&self) -> Result<MemorySnapshot, AIError> {
        let data = self.data.read().await;
        Ok(Self::build_snapshot(&data))
    }

    async fn drain_pending_sync(&self) -> Result<MemorySyncPayload, AIError> {
        let mut data = self.data.write().await;
        let payload = MemorySyncPayload {
            base_revision: data.last_synced_revision,
            next_revision: data.revision,
            operations: data.pending_sync.clone(),
        };
        data.last_synced_revision = data.revision;
        data.pending_sync.clear();
        Ok(payload)
    }

    async fn append_conversation_turn(&self, role: String, content: String) -> Result<(), AIError> {
        let content = Self::normalize_content(&content);
        if content.is_empty() {
            return Ok(());
        }

        let role = Self::normalize_role(&role);
        let mut data = self.data.write().await;
        data.conversation_history
            .push(ConversationTurn::new(role, content));
        if data.conversation_history.len() > MAX_CONVERSATION_TURNS {
            let overflow = data.conversation_history.len() - MAX_CONVERSATION_TURNS;
            data.conversation_history.drain(0..overflow);
        }
        drop(data);
        self.save_to_disk().await?;
        Ok(())
    }

    async fn get_recent_conversation_context(&self, limit: usize) -> Result<String, AIError> {
        let data = self.data.read().await;
        if data.conversation_history.is_empty() {
            return Ok(String::new());
        }

        let take = limit.max(1).min(MAX_CONVERSATION_CONTEXT_TURNS);
        let lines = data
            .conversation_history
            .iter()
            .rev()
            .take(take)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|turn| {
                let label = if turn.role == "assistant" {
                    "助手"
                } else {
                    "用户"
                };
                format!("{}: {}", label, turn.content)
            })
            .collect::<Vec<_>>();
        Ok(lines.join("\n"))
    }
}
