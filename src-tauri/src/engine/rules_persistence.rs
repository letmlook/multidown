//! 分类规则持久化

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_rules_preserve_valid_records_and_quarantine_invalid_and_duplicate_ids() {
        let dir = std::env::temp_dir().join(format!("rules-recovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rules.json");
        let original = r#"[{"id":"valid","name":"Media","match_type":"extension","patterns":["mp4"],"save_path":"/media","enabled":true,"priority":0},{"id":"bad","name":"Invalid","match_type":"unknown","patterns":[],"save_path":"","enabled":true,"priority":1},{"id":"valid","name":"Duplicate","match_type":"domain","patterns":["example.com"],"save_path":"/other","enabled":false,"priority":2},42]"#;
        std::fs::write(&path, original).unwrap();
        let report = load_rules_report(&path).unwrap();
        assert_eq!(report.data.len(), 1);
        assert_eq!(report.data[0].name, "Media");
        assert_eq!(report.warnings.len(), 3);
        assert!(report.migrated);
        assert_eq!(report.schema_version, 0);
        let quarantined: serde_json::Value = serde_json::from_slice(&std::fs::read(report.recovery_path.unwrap()).unwrap()).unwrap();
        assert_eq!(quarantined[0]["value"]["match_type"], "unknown");
        assert_eq!(quarantined[1]["value"]["name"], "Duplicate");
        assert_eq!(quarantined[2]["value"], 42);
        assert_eq!(std::fs::read_to_string(path.with_extension("json.bak")).unwrap(), original);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn future_schema_is_rejected_without_rewriting_file() {
        let dir = std::env::temp_dir().join(format!("rules-future-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rules.json");
        let original = r#"{"schema_version":2,"written_at":"2026-10-03T00:00:00Z","data":[]}"#;
        std::fs::write(&path, original).unwrap();
        assert!(matches!(load_rules_report(&path), Err(StoreError::UnsupportedVersion(2))));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn legacy_rule_array_preserves_ids_in_versioned_store() {
        let dir = std::env::temp_dir().join(format!("rules-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rules.json");
        std::fs::write(&path, r#"[{"id":"rule-1","name":"Media","match_type":"domain","patterns":["example.com"],"save_path":"/media","enabled":true,"priority":2}]"#).unwrap();
        let rules = load_rules(&path).unwrap();
        assert_eq!(rules[0].id, "rule-1");
        save_rules(&path, &rules).await.unwrap();
        let disk: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(disk["schema_version"], 1);
        assert_eq!(disk["data"][0]["match_type"], "domain");
        assert_eq!(load_rules(&path).unwrap()[0].save_path, "/media");
        std::fs::remove_dir_all(dir).unwrap();
    }
}


use crate::engine::rules::{CategoryRule, MatchType};
use std::path::Path;
use crate::storage::{load_store, save_store, LoadReport, RecoveryWarning, StoreError};
use serde_json::Value;

/// Decode records independently so one rejected record cannot discard its peers.
pub(crate) fn load_records<T: serde::Serialize>(
    path: &Path,
    domain: &'static str,
    decode: impl Fn(Value) -> Result<T, String>,
) -> Result<LoadReport<Vec<T>>, StoreError> {
    load_store(path, domain, |version, data| {
        if version > 1 {
            return Err(StoreError::UnsupportedVersion(version));
        }
        let records = data.as_array().ok_or_else(|| StoreError::InvalidEnvelope(format!("{domain} data must be an array")))?;
        let mut valid = Vec::new();
        let mut warnings = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (index, value) in records.iter().enumerate() {
            let key = value.get("id").and_then(Value::as_str).filter(|id| !id.trim().is_empty());
            let parsed = match key {
                None => Err("missing or empty record ID".into()),
                Some(id) if seen.contains(id) => Err("duplicate record ID".into()),
                Some(_) => decode(value.clone()),
            };
            match parsed {
                Ok(record) => {
                    seen.insert(key.unwrap().to_owned());
                    valid.push(record);
                }
                Err(message) => warnings.push(RecoveryWarning {
                    id: format!("{domain}-record-{index}"),
                    domain: domain.into(),
                    message,
                    recovery_path: None,
                    record_key: Some(key.map(str::to_owned).unwrap_or_else(|| index.to_string())),
                    rejected_value: Some(value.clone()),
                }),
            }
        }
        Ok((valid, warnings))
    })
}

#[allow(dead_code)]
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

#[allow(dead_code)]
pub fn rules_path(app_data_dir: &Path) -> std::path::PathBuf {
    app_data_dir.join(RULES_FILENAME)
}

pub fn load_rules(path: &Path) -> Result<Vec<CategoryRule>, Box<dyn std::error::Error + Send + Sync>> {
    if !path.exists() {
        return Ok(default_rules());
    }
    Ok(load_rules_report(path)?.data)
}

pub fn load_rules_report(path: &Path) -> Result<LoadReport<Vec<CategoryRule>>, StoreError> {
    load_records(path, "rules", |value| {
        let dto: RuleDto = serde_json::from_value(value).map_err(|error| error.to_string())?;
        if !matches!(dto.match_type.as_str(), "extension" | "domain" | "mime_type" | "url_contains") {
            return Err(format!("unknown match type: {}", dto.match_type));
        }
        Ok(CategoryRule::from(&dto))
    })
}

pub async fn save_rules(path: &Path, rules: &[CategoryRule]) -> Result<(), std::io::Error> {
    let dtos: Vec<RuleDto> = rules.iter().map(RuleDto::from).collect();
    save_store(path, 1, &dtos).map_err(std::io::Error::other)
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
