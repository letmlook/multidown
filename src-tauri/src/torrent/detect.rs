//! 协议嗅探与磁力链接解析。
//!
//! 刻意不依赖 BT 引擎：这些是纯函数，既服务于"建任务之前"的输入判定，
//! 也能用零成本的单元测试覆盖住边界情况。

/// 用户输入（或 URL）应该走哪条下载路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputProtocol {
    /// 普通 HTTP/HTTPS 下载
    Http,
    /// `magnet:?xt=...`
    Magnet,
    /// 本地 `.torrent` 文件路径（或 `file://` URL）
    TorrentFile,
    /// 指向 `.torrent` 的 http(s) URL
    TorrentUrl,
}

impl InputProtocol {
    pub fn is_torrent(self) -> bool {
        !matches!(self, InputProtocol::Http)
    }
}

/// 嗅探输入应该走哪条路径。
///
/// 判定顺序很重要：`magnet:` 优先于 http，`.torrent` 后缀优先于普通 http。
pub fn sniff(input: &str) -> InputProtocol {
    let s = input.trim();
    let lower = s.to_ascii_lowercase();

    if is_magnet(&lower) {
        return InputProtocol::Magnet;
    }
    if lower.starts_with("file://") {
        return if lower.ends_with(".torrent") {
            InputProtocol::TorrentFile
        } else {
            InputProtocol::Http
        };
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return if url_path_ends_with(&lower, ".torrent") {
            InputProtocol::TorrentUrl
        } else {
            InputProtocol::Http
        };
    }
    if lower.ends_with(".torrent") {
        return InputProtocol::TorrentFile;
    }
    InputProtocol::Http
}

pub fn is_magnet(lowercased: &str) -> bool {
    lowercased.starts_with("magnet:?")
}

/// 判断 URL 的 path 部分（忽略 query/fragment）是否以给定后缀结尾。
///
/// 不能直接对整串 `ends_with`，否则 `https://x/y.zip?name=a.torrent` 会被误判。
fn url_path_ends_with(url: &str, suffix: &str) -> bool {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let path = after_scheme
        .split(['?', '#'])
        .next()
        .unwrap_or(after_scheme);
    path.ends_with(suffix)
}

/// 解析后的磁力链接信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagnetInfo {
    /// v1 info hash，40 位小写 hex
    pub info_hash: String,
    pub display_name: Option<String>,
    pub trackers: Vec<String>,
    /// BEP 53 `so=` 选中的文件索引（支持 `0,2,4-7` 这类区间写法）
    pub select_only: Option<Vec<usize>>,
}

impl MagnetInfo {
    /// 元数据就绪前用于任务名的占位符。
    pub fn placeholder_filename(&self) -> String {
        match &self.display_name {
            Some(name) if !name.trim().is_empty() => sanitize_filename(name),
            // 没有 dn 时用 info hash 前 16 位，保证同一资源每次生成的名字稳定
            _ => format!("torrent-{}", &self.info_hash[..self.info_hash.len().min(16)]),
        }
    }
}

/// 解析磁力链接。
///
/// 只支持 v1（`urn:btih:`）——纯 v2 的 `urn:btmh:` 会返回明确的错误，
/// 而不是静默失败（BT 引擎同样只支持 v1）。
pub fn parse_magnet(input: &str) -> Result<MagnetInfo, String> {
    let s = input.trim();
    let lower = s.to_ascii_lowercase();
    if !is_magnet(&lower) {
        return Err("不是合法的磁力链接（应以 magnet:? 开头）".to_string());
    }

    // 去掉 "magnet:?" 前缀
    let query = &s["magnet:?".len()..];

    let mut info_hash: Option<String> = None;
    let mut display_name: Option<String> = None;
    let mut trackers: Vec<String> = Vec::new();
    let mut select_only_raw: Option<String> = None;
    let mut saw_btmh = false;

    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_val) = match pair.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        let key = raw_key.to_ascii_lowercase();
        let value = percent_decode(raw_val);
        match key.as_str() {
            "xt" => {
                // 可能带多个 xt（btih + btmh）；优先取 btih
                let v = value.trim();
                let v_lower = v.to_ascii_lowercase();
                if let Some(hash) = v_lower.strip_prefix("urn:btih:") {
                    if info_hash.is_none() {
                        info_hash = Some(normalize_info_hash(hash)?);
                    }
                } else if v_lower.starts_with("urn:btmh:") {
                    saw_btmh = true;
                }
            }
            "dn" => {
                if display_name.is_none() && !value.trim().is_empty() {
                    display_name = Some(value);
                }
            }
            "tr" => {
                if !value.trim().is_empty() {
                    trackers.push(value);
                }
            }
            "so" if select_only_raw.is_none() => {
                select_only_raw = Some(value);
            }
            _ => {}
        }
    }

    let info_hash = match info_hash {
        Some(h) => h,
        None if saw_btmh => {
            return Err(
                "该磁力链接只包含 BitTorrent v2 (urn:btmh:) 的 info hash，当前引擎只支持 v1，无法下载"
                    .to_string(),
            )
        }
        None => return Err("磁力链接缺少 xt=urn:btih: 参数".to_string()),
    };

    let select_only = select_only_raw.as_deref().and_then(parse_select_only);

    Ok(MagnetInfo {
        info_hash,
        display_name,
        trackers,
        select_only,
    })
}

/// 规范化 info hash：接受 40 位 hex 或 32 位 base32，统一产出小写 hex。
fn normalize_info_hash(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.len() == 40 {
        if raw.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(raw.to_ascii_lowercase());
        }
        return Err("磁力链接的 info hash 含非 16 进制字符".to_string());
    }
    if raw.len() == 32 {
        let bytes = base32_decode(raw)
            .ok_or_else(|| "磁力链接的 base32 info hash 解码失败".to_string())?;
        return Ok(hex_encode(&bytes));
    }
    Err(format!(
        "磁力链接的 info hash 长度应为 40(hex) 或 32(base32)，实际为 {}",
        raw.len()
    ))
}

/// RFC 4648 base32（不含 padding）。
fn base32_decode(input: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = Vec::with_capacity(input.len() * 5 / 8);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for ch in input.bytes() {
        let upper = ch.to_ascii_uppercase();
        let value = ALPHABET.iter().position(|&c| c == upper)? as u32;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xFF) as u8);
        }
    }
    (!out.is_empty()).then_some(out)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0F) as usize] as char);
    }
    s
}

/// 解析 BEP 53 的 `so=` 参数：逗号分隔的索引与区间，如 `0,2,4-7`。
fn parse_select_only(raw: &str) -> Option<Vec<usize>> {
    let mut out: Vec<usize> = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once('-') {
            Some((a, b)) => {
                let (Ok(a), Ok(b)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) else {
                    continue;
                };
                if a > b {
                    continue;
                }
                // 防御异常大的区间（磁力链接是外部输入）
                if b - a > 10_000 {
                    continue;
                }
                out.extend(a..=b);
            }
            None => {
                if let Ok(idx) = part.parse::<usize>() {
                    out.push(idx);
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    (!out.is_empty()).then_some(out)
}

/// 百分号解码；`+` 按查询串惯例解码为空格。
fn percent_decode(input: &str) -> String {
    let plus_normalized = input.replace('+', " ");
    urlencoding::decode(&plus_normalized)
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| plus_normalized.clone())
}

/// 去掉文件名里在目标平台上非法的字符，并防止路径穿越。
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    // 折叠 ".." 与首尾的点，避免 `dn` 里塞入相对路径
    let collapsed = cleaned.replace("..", "_");
    let trimmed = collapsed.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "torrent".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 把 `file://` URL 转成本地路径（macOS 的 RunEvent::Opened 给的是 file URL）。
///
/// - `file:///Users/x/a.torrent` → 空 host，去掉前缀后整串就是路径
/// - `file://localhost/Users/x/a.torrent` → 非空 host，取第一个 `/` 之后的部分
///
/// 注意这里**不做** `+` → 空格转换：URL 的 path 段里 `+` 是字面加号，
/// 只有 query 段才有 `+` 表示空格的惯例。
pub fn file_url_to_path(url: &str) -> Option<std::path::PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let decoded = if rest.starts_with('/') {
        // 空 host：file:///path
        percent_decode_path(rest)
    } else {
        // 非空 host：file://host/path
        let (_host, path) = rest.split_once('/')?;
        percent_decode_path(&format!("/{path}"))
    };
    (!decoded.is_empty()).then(|| std::path::PathBuf::from(decoded))
}

/// 只做百分号解码（不做 `+` → 空格），用于 URL 的 path 段。
fn percent_decode_path(input: &str) -> String {
    urlencoding::decode(input)
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| input.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_magnet_before_http() {
        assert_eq!(
            sniff("magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862"),
            InputProtocol::Magnet
        );
        assert_eq!(sniff("MAGNET:?xt=urn:btih:aa"), InputProtocol::Magnet);
    }

    #[test]
    fn sniffs_torrent_url_ignoring_query_and_fragment() {
        assert_eq!(
            sniff("https://example.com/a/ubuntu.torrent"),
            InputProtocol::TorrentUrl
        );
        assert_eq!(
            sniff("https://example.com/a/UBUNTU.TORRENT"),
            InputProtocol::TorrentUrl
        );
        // query 里有 .torrent 但路径是 .zip → 仍然是普通 HTTP
        assert_eq!(
            sniff("https://example.com/f.zip?name=a.torrent"),
            InputProtocol::Http
        );
        assert_eq!(
            sniff("https://example.com/a.torrent#frag"),
            InputProtocol::TorrentUrl
        );
    }

    #[test]
    fn sniffs_local_torrent_paths() {
        assert_eq!(sniff("/Users/x/ubuntu.torrent"), InputProtocol::TorrentFile);
        assert_eq!(
            sniff("file:///Users/x/ubuntu.torrent"),
            InputProtocol::TorrentFile
        );
        assert_eq!(sniff("C:\\dl\\ubuntu.torrent"), InputProtocol::TorrentFile);
        assert_eq!(sniff("/Users/x/ubuntu.zip"), InputProtocol::Http);
    }

    #[test]
    fn parses_hex_magnet() {
        let m = parse_magnet(
            "magnet:?xt=urn:btih:CAB507494D02EBB1178B38F2E9D7BE299C86B862&dn=ubuntu.iso&tr=udp%3A%2F%2Ftracker.example%3A80%2Fannounce",
        )
        .unwrap();
        assert_eq!(m.info_hash, "cab507494d02ebb1178b38f2e9d7be299c86b862");
        assert_eq!(m.display_name.as_deref(), Some("ubuntu.iso"));
        assert_eq!(m.trackers, vec!["udp://tracker.example:80/announce"]);
        assert_eq!(m.select_only, None);
    }

    #[test]
    fn decodes_base32_info_hash() {
        // 已知向量：base32 "MZXW6YTBOI" == "foobar"
        assert_eq!(
            hex_encode(&base32_decode("MZXW6YTBOI").unwrap()),
            "666f6f626172"
        );

        // 32 位 base32 的 info hash 必须规范化为 40 位 hex，
        // 且与等价的 hex 写法解析出同一个值（Ubuntu 21.04 live-server）
        let from_base32 =
            parse_magnet("magnet:?xt=urn:btih:ZK2QOSKNALV3CF4LHDZOTV56FGOINODC").unwrap();
        let from_hex = parse_magnet(
            "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862",
        )
        .unwrap();
        assert_eq!(from_base32.info_hash.len(), 40);
        assert_eq!(from_base32.info_hash, from_hex.info_hash);
        assert_eq!(from_base32.info_hash, "cab507494d02ebb1178b38f2e9d7be299c86b862");
    }

    #[test]
    fn parses_multiple_trackers_and_plus_decoding() {
        let m = parse_magnet(
            "magnet:?xt=urn:btih:0000000000000000000000000000000000000000&dn=a+b&tr=udp://a:1&tr=udp://b:2",
        )
        .unwrap();
        assert_eq!(m.display_name.as_deref(), Some("a b"));
        assert_eq!(m.trackers, vec!["udp://a:1", "udp://b:2"]);
    }

    #[test]
    fn parses_select_only_ranges() {
        assert_eq!(parse_select_only("0,2,4-7"), Some(vec![0, 2, 4, 5, 6, 7]));
        assert_eq!(parse_select_only("3"), Some(vec![3]));
        assert_eq!(parse_select_only(""), None);
        assert_eq!(parse_select_only("abc"), None);
        // 反向区间忽略
        assert_eq!(parse_select_only("7-4"), None);
        let m = parse_magnet(
            "magnet:?xt=urn:btih:0000000000000000000000000000000000000000&so=1-2",
        )
        .unwrap();
        assert_eq!(m.select_only, Some(vec![1, 2]));
    }

    #[test]
    fn rejects_v2_only_magnet_with_clear_message() {
        let err = parse_magnet("magnet:?xt=urn:btmh:1220caf1e1c30e81cb361b9ee167c4aa64228a7fa4fa9f6105232b28ad099f3a302e")
            .unwrap_err();
        assert!(err.contains("v2"), "错误信息应说明 v2 不支持，实际为: {err}");
    }

    #[test]
    fn rejects_invalid_magnets() {
        assert!(parse_magnet("https://example.com/a.torrent").is_err());
        assert!(parse_magnet("magnet:?dn=no-xt").is_err());
        assert!(parse_magnet("magnet:?xt=urn:btih:zzzz").is_err());
    }

    #[test]
    fn prefers_btih_when_both_xt_present() {
        let m = parse_magnet(
            "magnet:?xt=urn:btmh:1220aa&xt=urn:btih:0000000000000000000000000000000000000001",
        )
        .unwrap();
        assert_eq!(m.info_hash, "0000000000000000000000000000000000000001");
    }

    #[test]
    fn placeholder_filename_is_stable_and_safe() {
        let with_dn = parse_magnet(
            "magnet:?xt=urn:btih:0000000000000000000000000000000000000000&dn=../../etc/passwd",
        )
        .unwrap();
        let name = with_dn.placeholder_filename();
        assert!(!name.contains('/'), "路径分隔符必须被清理: {name}");
        assert!(!name.contains(".."), "不应保留 .. : {name}");

        let without_dn = parse_magnet(
            "magnet:?xt=urn:btih:abcdef0000000000000000000000000000000000",
        )
        .unwrap();
        assert_eq!(
            without_dn.placeholder_filename(),
            "torrent-abcdef0000000000"
        );
    }

    #[test]
    fn converts_file_urls_to_paths() {
        assert_eq!(
            file_url_to_path("file:///Users/x/a.torrent").unwrap(),
            std::path::PathBuf::from("/Users/x/a.torrent")
        );
        assert_eq!(
            file_url_to_path("file:///Users/x/a%20b.torrent").unwrap(),
            std::path::PathBuf::from("/Users/x/a b.torrent")
        );
        // 非空 host 形式
        assert_eq!(
            file_url_to_path("file://localhost/Users/x/a.torrent").unwrap(),
            std::path::PathBuf::from("/Users/x/a.torrent")
        );
        // path 段里的 '+' 是字面加号，不能被当作空格
        assert_eq!(
            file_url_to_path("file:///Users/x/a+b.torrent").unwrap(),
            std::path::PathBuf::from("/Users/x/a+b.torrent")
        );
        assert!(file_url_to_path("https://x/a.torrent").is_none());
    }
}
