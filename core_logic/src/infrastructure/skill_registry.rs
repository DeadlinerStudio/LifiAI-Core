use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::models::{AIError, AgentRole, ToolMetadata};

#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    skills: Vec<ToolMetadata>,
}

impl SkillRegistry {
    pub fn empty() -> Self {
        Self { skills: Vec::new() }
    }

    pub fn from_skills(skills: Vec<ToolMetadata>) -> Self {
        Self { skills }
    }

    pub fn load_from_dir(dir: impl AsRef<Path>) -> Result<Self, AIError> {
        let dir = dir.as_ref();
        if !dir.exists() {
            return Ok(Self::empty());
        }

        let mut files = Vec::new();
        Self::collect_json_files(dir, &mut files)?;

        let mut skills = Vec::new();
        for file in files {
            let content = fs::read_to_string(&file).map_err(|err| {
                AIError::ConfigError(format!("无法读取 skill 文件 {}: {}", file.display(), err))
            })?;
            let metadata = serde_json::from_str::<ToolMetadata>(&content).map_err(|err| {
                AIError::ConfigError(format!("无法解析 skill 文件 {}: {}", file.display(), err))
            })?;
            skills.push(metadata);
        }

        Ok(Self { skills })
    }

    pub fn for_role_and_platform(&self, role: &AgentRole, platform: &str) -> Vec<ToolMetadata> {
        self.skills
            .iter()
            .filter(|skill| &skill.domain == role)
            .filter(|skill| {
                skill.supported_platforms.is_empty()
                    || skill
                        .supported_platforms
                        .iter()
                        .any(|supported| supported.eq_ignore_ascii_case(platform))
            })
            .cloned()
            .collect()
    }

    pub fn find_for_role_and_platform(
        &self,
        role: &AgentRole,
        platform: &str,
        tool_name: &str,
    ) -> Option<ToolMetadata> {
        self.for_role_and_platform(role, platform)
            .into_iter()
            .find(|skill| skill.name == tool_name)
    }

    fn collect_json_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), AIError> {
        let entries = fs::read_dir(dir).map_err(|err| {
            AIError::ConfigError(format!("无法读取 skill 目录 {}: {}", dir.display(), err))
        })?;

        for entry in entries {
            let entry = entry.map_err(|err| {
                AIError::ConfigError(format!("遍历 skill 目录 {} 失败: {}", dir.display(), err))
            })?;
            let path = entry.path();
            if path.is_dir() {
                Self::collect_json_files(&path, files)?;
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                files.push(path);
            }
        }

        files.sort();
        Ok(())
    }
}
