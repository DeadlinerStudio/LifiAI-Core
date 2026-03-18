use async_trait::async_trait;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};
use crate::domain::models::{MemoryFragment, AIError, MAX_MEMORY_FRAGMENTS, MAX_MEMORY_AGE_DAYS};
use crate::domain::traits::MemoryStore;
use chrono::{Utc, Duration};

#[derive(Serialize, Deserialize, Default)]
struct StorageData {
    fragments: Vec<MemoryFragment>,
    user_profile: String,
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

        // 1) 时间过期淘汰
        data.fragments.retain(|f| f.timestamp >= cutoff);

        // 2) 超容量淘汰
        if data.fragments.len() > MAX_MEMORY_FRAGMENTS {
            data.fragments.sort_by(|a, b| {
                if a.importance != b.importance {
                    b.importance.cmp(&a.importance)
                } else {
                    b.timestamp.cmp(&a.timestamp)
                }
            });
            data.fragments.truncate(MAX_MEMORY_FRAGMENTS);
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
}
