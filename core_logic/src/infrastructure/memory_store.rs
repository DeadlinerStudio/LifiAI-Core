use async_trait::async_trait;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};
use crate::domain::models::{
    AIError, MemoryFragment, MemorySnapshot, MemorySyncOperation, MemorySyncPayload,
    MAX_MEMORY_AGE_DAYS, MAX_MEMORY_FRAGMENTS,
};
use crate::domain::traits::MemoryStore;
use chrono::{Utc, Duration};

#[derive(Serialize, Deserialize, Default)]
struct StorageData {
    revision: u64,
    last_synced_revision: u64,
    fragments: Vec<MemoryFragment>,
    user_profile: String,
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
        let fragment = data.fragments.last().cloned().ok_or(AIError::ParsingFailed)?;
        data.revision += 1;
        data.pending_sync.push(MemorySyncOperation::UpsertFragment { fragment });
        
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
        data.pending_sync.push(MemorySyncOperation::ReplaceUserProfile {
            profile,
        });
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
            let bullets: Vec<String> = data.fragments.iter()
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
}
