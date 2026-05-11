//! 分类规则：根据扩展名、域名、MIME 类型等自动将下载文件保存到对应目录

use crate::engine::types::MatchResult;
use serde::{Deserialize, Serialize};

/// 匹配类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchType {
    Extension,    // 文件扩展名，如 "mp4", "avi"
    Domain,        // 域名，如 "youtube.com"
    MimeType,      // MIME类型，如 "video/*"
    UrlContains,   // URL包含字符串
}

/// 单条分类规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryRule {
    pub id: String,
    pub name: String,           // 规则名称，如"视频"
    pub match_type: MatchType,  // 匹配类型
    pub patterns: Vec<String>,   // 匹配模式列表
    pub save_path: String,      // 目标保存路径
    pub enabled: bool,
    pub priority: usize,        // 优先级，数值越小越靠前
}

impl CategoryRule {
    /// 判断规则是否匹配给定的 URL、文件名和 MIME
    pub fn matches(&self, url: &str, filename: &str, mime: Option<&str>) -> bool {
        if !self.enabled {
            return false;
        }
        for pat in &self.patterns {
            if pat.is_empty() {
                continue;
            }
            match self.match_type {
                MatchType::Extension => {
                    // 匹配文件扩展名（pat 不带点，如 "mp4"）
                    if filename.rsplit('.').next().map(|e| e.eq_ignore_ascii_case(pat)).unwrap_or(false) {
                        return true;
                    }
                }
                MatchType::Domain => {
                    // 匹配域名：检查 URL 是否包含该域名
                    if url.contains(pat) {
                        return true;
                    }
                }
                MatchType::MimeType => {
                    if let Some(m) = mime {
                        // 简单 glob 匹配：支持 video/* 形式
                        if pat.ends_with("/*") {
                            let prefix = &pat[..pat.len() - 2];
                            if m.starts_with(prefix) {
                                return true;
                            }
                        } else if m.eq_ignore_ascii_case(pat) {
                            return true;
                        }
                    }
                }
                MatchType::UrlContains => {
                    if url.to_lowercase().contains(&pat.to_lowercase()) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// 解析保存路径中的变量：{category}、{filename}、{date}
    pub fn resolve_save_path(&self, filename: &str) -> String {
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let category = &self.name;
        self.save_path
            .replace("{category}", category)
            .replace("{filename}", filename)
            .replace("{date}", &date)
    }
}

/// 根据规则列表，为给定 URL/文件名找到一个匹配的保存路径
/// 若无匹配返回 None（使用默认路径）
pub fn match_rule(rules: &[CategoryRule], url: &str, filename: &str, mime: Option<&str>) -> Option<String> {
    match_rule_info(rules, url, filename, mime).map(|r| r.save_dir)
}

/// Like match_rule but returns full MatchResult for test_rules command
pub fn match_rule_info(rules: &[CategoryRule], url: &str, filename: &str, mime: Option<&str>) -> Option<MatchResult> {
    let mut sorted: Vec<_> = rules.iter().collect();
    sorted.sort_by_key(|r| r.priority);
    for rule in sorted {
        for pat in &rule.patterns {
            if pat.is_empty() { continue; }
            let matched = match rule.match_type {
                MatchType::Extension => {
                    filename.rsplit('.').next().map(|e| e.eq_ignore_ascii_case(pat)).unwrap_or(false)
                }
                MatchType::Domain => url.contains(pat),
                MatchType::MimeType => {
                    if let Some(m) = mime {
                        if pat.ends_with("/*") {
                            let prefix = &pat[..pat.len() - 2];
                            m.starts_with(prefix)
                        } else {
                            m.eq_ignore_ascii_case(pat)
                        }
                    } else { false }
                }
                MatchType::UrlContains => url.to_lowercase().contains(&pat.to_lowercase()),
            };
            if matched {
                return Some(MatchResult {
                    rule_id: rule.id.clone(),
                    category: rule.name.clone(),
                    save_dir: rule.resolve_save_path(filename),
                    matched_pattern: pat.clone(),
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rule(match_type: MatchType, patterns: Vec<&str>, save_path: &str) -> CategoryRule {
        CategoryRule {
            id: "1".into(),
            name: "视频".into(),
            match_type,
            patterns: patterns.into_iter().map(|s| s.into()).collect(),
            save_path: save_path.into(),
            enabled: true,
            priority: 0,
        }
    }

    #[test]
    fn test_extension_match() {
        let rule = make_rule(MatchType::Extension, vec!["mp4", "avi", "mkv"], "视频/");
        assert!(rule.matches("", "test.mp4", None));
        assert!(rule.matches("", "test.AVI", None));
        assert!(!rule.matches("", "test.txt", None));
    }

    #[test]
    fn test_domain_match() {
        let rule = make_rule(MatchType::Domain, vec!["youtube.com", "bilibili.com"], "视频/");
        assert!(rule.matches("https://www.youtube.com/watch?v=123", "x.mp4", None));
        assert!(rule.matches("https://www.bilibili.com/video/av1", "x.mp4", None));
        assert!(!rule.matches("https://www.google.com/", "x.mp4", None));
    }

    #[test]
    fn test_resolve_path() {
        let rule = CategoryRule {
            id: "1".into(),
            name: "视频".into(),
            match_type: MatchType::Extension,
            patterns: vec!["mp4".into()],
            save_path: "{category}/2024/{filename}".into(),
            enabled: true,
            priority: 0,
        };
        assert_eq!(rule.resolve_save_path("test.mp4"), "视频/2024/test.mp4");
    }
}