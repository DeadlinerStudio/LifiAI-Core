use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::models::{
    AIError, AgentRole, CapabilityExecutionMode, ResolvedToolCandidate, ToolCapability,
    ToolExecutionPlan, ToolMetadata,
};

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
            return Err(AIError::ConfigError(format!(
                "Skill 目录不存在: {}",
                dir.display()
            )));
        }

        let mut files = Vec::new();
        Self::collect_json_files(dir, &mut files)?;

        if files.is_empty() {
            return Err(AIError::ConfigError(format!(
                "Skill 目录为空: {}",
                dir.display()
            )));
        }

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

    pub fn candidates_for_capability(
        &self,
        role: &AgentRole,
        platform: &str,
        capability: &ToolCapability,
    ) -> Vec<ResolvedToolCandidate> {
        self.for_role_and_platform(role, platform)
            .into_iter()
            .filter(|skill| skill.capabilities.iter().any(|item| item == capability))
            .map(|tool| ResolvedToolCandidate {
                capability: capability.clone(),
                tool,
            })
            .collect()
    }

    pub fn resolve_execution_plan(
        &self,
        role: &AgentRole,
        platform: &str,
        capability: &ToolCapability,
        args: serde_json::Value,
        execution_mode: Option<CapabilityExecutionMode>,
    ) -> Result<ToolExecutionPlan, AIError> {
        let mut candidates = self.candidates_for_capability(role, platform, capability);
        candidates.sort_by(|a, b| a.tool.name.cmp(&b.tool.name));

        match candidates.len() {
            1 => {
                let candidate = candidates.remove(0);
                let resolved_execution_mode = execution_mode.unwrap_or_else(|| {
                    if candidate.tool.requires_ui_confirmation {
                        CapabilityExecutionMode::AskUser
                    } else {
                        CapabilityExecutionMode::Auto
                    }
                });
                Ok(ToolExecutionPlan {
                    capability: capability.clone(),
                    tool_name: candidate.tool.name,
                    args,
                    requires_ui_confirmation: candidate.tool.requires_ui_confirmation,
                    execution_mode: resolved_execution_mode,
                })
            }
            0 => Err(AIError::ConfigError(format!(
                "{} 在平台 {} 下未配置 capability {:?} 对应工具",
                role.as_label(),
                platform,
                capability
            ))),
            _ => Err(AIError::ConfigError(format!(
                "{} 在平台 {} 下存在多个 capability {:?} 对应工具，无法唯一解析",
                role.as_label(),
                platform,
                capability
            ))),
        }
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

#[cfg(test)]
mod tests;
