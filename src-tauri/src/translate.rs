//! Dịch text trước khi đọc. Thay deep-translator bằng endpoint web của Google Translate.
//! Passthrough (trả nguyên văn) nếu tắt hoặc lỗi mạng — không bao giờ chặn luồng đọc.

use std::time::Duration;

pub struct Translator {
    pub enabled: bool,
    source: String,
    target: String,
    client: reqwest::blocking::Client,
}

impl Translator {
    pub fn new(enabled: bool, source: &str, target: &str) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        Self { enabled, source: source.into(), target: target.into(), client }
    }

    pub fn translate(&self, text: &str) -> String {
        if !self.enabled || text.trim().is_empty() {
            return text.to_string();
        }
        let url = format!(
            "https://translate.googleapis.com/translate_a/single?client=gtx&sl={}&tl={}&dt=t&q={}",
            self.source,
            self.target,
            urlencoding::encode(text)
        );
        match self.client.get(&url).send().and_then(|r| r.text()) {
            Ok(body) => parse_response(&body).unwrap_or_else(|| text.to_string()),
            Err(e) => {
                eprintln!("[translate] lỗi (trả nguyên văn): {e}");
                text.to_string()
            }
        }
    }
}

/// Body dạng `[[["đã dịch","gốc",null,null,10],[...]],null,"en",...]` -> nối các đoạn đã dịch.
fn parse_response(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let chunks = v.get(0)?.as_array()?;
    let out: String = chunks.iter().filter_map(|c| c.get(0)?.as_str()).collect();
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghep_nhieu_doan() {
        let body = r#"[[["Xin chào ","Hello ",null,null,10],["thế giới","world",null,null,10]],null,"en"]"#;
        assert_eq!(parse_response(body).as_deref(), Some("Xin chào thế giới"));
    }

    #[test]
    fn body_hong_tra_none() {
        assert!(parse_response("not json").is_none());
        assert!(parse_response("[null,null,\"en\"]").is_none());
        assert!(parse_response(r#"[[["   ","x",null,null,10]],null,"en"]"#).is_none());
    }

    #[test]
    fn tat_dich_tra_nguyen_van() {
        let t = Translator::new(false, "en", "vi");
        assert_eq!(t.translate("Hello"), "Hello");
    }
}
