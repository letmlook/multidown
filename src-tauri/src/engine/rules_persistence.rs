//! 分类规则持久化

use crate::engine::rules::{CategoryRule, MatchType};
use std::path::Path;

const RULES_FILENAME: &str = "category_rules.json";

/// 简化版结构用于序列化（MatchType 是 ctor name）
#[derive(serde::Serialize, serde::Deserialize)]
struct RuleDto {
    id: String,
    name: String,
    match_type: String,
    patterns: Vec<String>,
    save_path: String,
    enabled: bool,
    priority: usize,
}

impl From<&CategoryRule> for RuleDto {
    fn from(r: &CategoryRule) -> Self {
        let match_type = match r.match_type {
            MatchType::Extension => "extension",
            MatchType::Domain => "domain",
            MatchType::MimeType => "mime_type",
            MatchType::UrlContains => "url_contains",
        };
        Self {
            id: r.id.clone(),
            name: r.name.clone(),
            match_type: match_type.to_string(),
            patterns: r.patterns.clone(),
            save_path: r.save_path.clone(),
            enabled: r.enabled,
            priority: r.priority,
        }
    }
}

impl From<&RuleDto> for CategoryRule {
    fn from(dto: &RuleDto) -> Self {
        let match_type = match dto.match_type.as_str() {
            "extension" => MatchType::Extension,
            "domain" => MatchType::Domain,
            "mime_type" => MatchType::MimeType,
            "url_contains" => MatchType::UrlContains,
            _ => MatchType::Extension,
        };
        Self {
            id: dto.id.clone(),
            name: dto.name.clone(),
            match_type,
            patterns: dto.patterns.clone(),
            save_path: dto.save_path.clone(),
            enabled: dto.enabled,
            priority: dto.priority,
        }
    }
}

pub fn rules_path(app_data_dir: &Path) -> std::path::PathBuf {
    app_data_dir.join(RULES_FILENAME)
}

pub fn load_rules(path: &Path) -> Result<Vec<CategoryRule>, Box<dyn std::error::Error + Send + Sync>> {
    if !path.exists() {
        return Ok(default_rules());
    }
    let json = std::fs::read_to_string(path)?;
    let dtos: Vec<RuleDto> = serde_json::from_str(&json)?;
    Ok(dtos.iter().map(|dto| CategoryRule::from(dto)).collect())
}

pub async fn save_rules(path: &Path, rules: &[CategoryRule]) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let dtos: Vec<RuleDto> = rules.iter().map(|r| RuleDto::from(r)).collect();
    let json = serde_json::to_string_pretty(&dtos)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    tokio::fs::write(path, json).await
}

/// 默认规则（首次使用时的预设值）
fn default_rules() -> Vec<CategoryRule> {
    vec![
        CategoryRule {
            id: uuid::Uuid::new_v4().to_string(),
            name: "视频".into(),
            match_type: MatchType::Extension,
            patterns: vec!["mp4".into(), "avi".into(), "mkv".into(), "mov".into(), "wmv".into(), "flv".into(), "webm".into()],
            save_path: "视频/".into(),
            enabled: true,
            priority: 0,
        },
        CategoryRule {
            id: uuid::Uuid::new_v4().to_string(),
            name: "音频".into(),
            match_type: MatchType::Extension,
            patterns: vec!["mp3".into(), "wav".into(), "flac".into(), "aac".into(), "ogg".into(), "m4a".into(), "wma".into()],
            save_path: "音频/".into(),
            enabled: true,
            priority: 1,
        },
        CategoryRule {
            id: uuid::Uuid::new_v4().to_string(),
            name: "压缩包".into(),
            match_type: MatchType::Extension,
            patterns: vec!["zip".into(), "rar".into(), "7z".into(), "tar".into(), "gz".into(), "bz2".into(), "xz".into()],
            save_path: "压缩包/".into(),
            enabled: true,
            priority: 2,
        },
        CategoryRule {
            id: uuid::Uuid::new_v4().to_string(),
            name: "图片".into(),
            match_type: MatchType::Extension,
            patterns: vec!["jpg".into(), "jpeg".into(), "png".into(), "gif".into(), "bmp".into(), "webp".into(), "svg".into(), "ico".into()],
            save_path: "图片/".into(),
            enabled: true,
            priority: 3,
        },
        CategoryRule {
            id: uuid::Uuid::new_v4().to_string(),
            name: "文档".into(),
            match_type: MatchType::Extension,
            patterns: vec!["pdf".into(), "doc".into(), "docx".into(), "xls".into(), "xlsx".into(), "ppt".into(), "pptx".into(), "txt".into(), "md".into()],
            save_path: "文档/".into(),
            enabled: true,
            priority: 4,
        },
        CategoryRule {
            id: uuid::Uuid::new_v4().to_string(),
            name: "安装包".into(),
            match_type: MatchType::Extension,
            patterns: vec!["exe".into(), "msi".into(), "dmg".into(), "pkg".into(), "deb".into(), "rpm".into(), "apk".into()],
            save_path: "安装包/".into(),
            enabled: true,
            priority: 5,
        },
    ]
}
