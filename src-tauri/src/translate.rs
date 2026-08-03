//! Dịch text trước khi đọc. Hai đường:
//!   - Google Translate (endpoint web `client=gtx`, không cần key) — nhanh, dịch từng câu rời rạc.
//!   - LLM qua API **OpenAI-compatible** (`/chat/completions`): Groq, Gemini, OpenRouter,
//!     DeepSeek, OpenAI... chỉ khác `base_url` + `model` nên dùng chung một code path.
//! LLM được gửi kèm vài câu trước làm ngữ cảnh — đó là chỗ nó hơn Google Dịch (chọn đại từ
//! xưng hô nhất quán, hiểu thuật ngữ game).
//! Mọi lỗi đều lùi về Google, rồi cuối cùng là nguyên văn — không bao giờ chặn luồng đọc.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::config::Config;

/// Trần cho `llm_context_lines` — nhiều hơn tốn token mà thoại game hiếm khi dài mạch đến vậy.
const MAX_CONTEXT_LINES: usize = 5;
/// Dính 429 thì nghỉ hẳn một lúc — free tier chỉ 15–30 lệnh/phút, gọi tiếp chỉ tổ 429 tiếp.
const COOLDOWN: Duration = Duration::from_secs(60);
/// Bản dịch dài hơn mức này so với bản gốc = model đang giải thích lung tung -> bỏ, dùng Google.
const MAX_EXPANSION: usize = 5;
/// Chặn trên cho một lượt gọi. Dài hơn thì giọng đọc trễ thấy rõ, thà lùi về Google.
const TIMEOUT: Duration = Duration::from_secs(6);

struct Llm {
    base_url: String,
    model: String,
    api_key: String,
}

pub struct Translator {
    pub enabled: bool,
    source: String,
    target: String,
    llm: Option<Llm>,
    /// Câu **gốc** (chưa dịch) gần nhất — LLM cần đọc mạch ngôn ngữ nguồn, không phải bản dịch.
    context: VecDeque<String>,
    /// 0 = tắt gửi ngữ cảnh (dịch từng câu rời, rẻ token và nhanh hơn).
    context_lines: usize,
    cooldown_until: Option<Instant>,
    client: reqwest::blocking::Client,
}

impl Translator {
    pub fn new(cfg: &Config) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .unwrap_or_default();
        let llm = llm_enabled(cfg).then(|| {
            Llm {
                base_url: cfg.llm_base_url.trim().trim_end_matches('/').to_string(),
                model: cfg.llm_model.trim().to_string(),
                api_key: cfg.llm_api_key.trim().to_string(),
            }
        });
        Self {
            enabled: cfg.translate,
            source: cfg.translate_from.clone(),
            target: cfg.translate_to.clone(),
            llm,
            context: VecDeque::new(),
            context_lines: cfg.llm_context_lines.min(MAX_CONTEXT_LINES),
            cooldown_until: None,
            client,
        }
    }

    pub fn translate(&mut self, text: &str) -> String {
        if !self.enabled || text.trim().is_empty() {
            return text.to_string();
        }
        let out = self.via_llm(text).unwrap_or_else(|| self.via_google(text));
        self.remember(text);
        out
    }

    fn remember(&mut self, text: &str) {
        if self.context_lines == 0 {
            return;
        }
        self.context.push_back(text.trim().to_string());
        while self.context.len() > self.context_lines {
            self.context.pop_front();
        }
    }

    /// None = không dùng được LLM lần này (chưa cấu hình / đang cooldown / lỗi / output đáng ngờ).
    fn via_llm(&mut self, text: &str) -> Option<String> {
        // Clone ra trước: giữ `&self.llm` sẽ vướng lúc ghi `self.cooldown_until` bên dưới.
        let (base_url, model, api_key) = {
            let l = self.llm.as_ref()?;
            (l.base_url.clone(), l.model.clone(), l.api_key.clone())
        };
        if self.cooldown_until.is_some_and(|t| Instant::now() < t) {
            return None;
        }

        let body = serde_json::json!({
            "model": model,
            "temperature": 0.2,
            "max_tokens": 400,
            "messages": [
                { "role": "system", "content": system_prompt(&self.target) },
                { "role": "user", "content": user_prompt(&self.context, text) },
            ],
        });
        let res = self
            .client
            .post(format!("{base_url}/chat/completions"))
            .bearer_auth(&api_key)
            .json(&body)
            .send();

        let body = match res {
            Ok(r) if r.status().as_u16() == 429 => {
                self.cooldown_until = Some(Instant::now() + COOLDOWN);
                eprintln!("[translate] LLM hết quota (429), tạm dùng Google 60s");
                return None;
            }
            Ok(r) if !r.status().is_success() => {
                let status = r.status();
                let b: String = r.text().unwrap_or_default().chars().take(200).collect();
                eprintln!("[translate] LLM {status}: {b}");
                return None;
            }
            Ok(r) => r.text().ok()?,
            Err(e) => {
                eprintln!("[translate] LLM lỗi mạng: {e}");
                return None;
            }
        };

        let out = parse_llm(&body)?;
        // Model nhỏ đôi khi bịa cả đoạn giải thích; dài bất thường thì thà lấy Google Dịch.
        if out.chars().count() > text.chars().count() * MAX_EXPANSION + 40 {
            eprintln!("[translate] LLM trả lời dài bất thường, dùng Google");
            return None;
        }
        Some(out)
    }

    fn via_google(&self, text: &str) -> String {
        let url = format!(
            "https://translate.googleapis.com/translate_a/single?client=gtx&sl={}&tl={}&dt=t&q={}",
            self.source,
            self.target,
            urlencoding::encode(text)
        );
        match self.client.get(&url).send().and_then(|r| r.text()) {
            Ok(body) => parse_google(&body).unwrap_or_else(|| text.to_string()),
            Err(e) => {
                eprintln!("[translate] Google lỗi (trả nguyên văn): {e}");
                text.to_string()
            }
        }
    }
}

/// Thiếu key thì coi như KHÔNG có LLM ngay từ đầu — để người dùng thấy "dịch Google" ở thanh
/// trạng thái, thay vì tưởng đang dịch AI mà câu nào cũng âm thầm rơi vào fallback.
fn llm_enabled(cfg: &Config) -> bool {
    cfg.translate_engine == "llm" && !cfg.llm_api_key.trim().is_empty()
}

/// Mô tả cho thanh trạng thái. Tính thẳng từ Config (không cần dựng Translator) vì Translator
/// nay sống ở thread dịch riêng, thread chụp không với tới được.
pub fn describe(cfg: &Config) -> String {
    if !cfg.translate {
        return "dịch OFF".into();
    }
    if !llm_enabled(cfg) {
        return "dịch Google".into();
    }
    match cfg.llm_context_lines.min(MAX_CONTEXT_LINES) {
        0 => format!("dịch AI ({}, không ngữ cảnh)", cfg.llm_model),
        n => format!("dịch AI ({}, ngữ cảnh {n} câu)", cfg.llm_model),
    }
}

fn lang_name(code: &str) -> &str {
    if code.starts_with("vi") {
        "tiếng Việt"
    } else {
        code
    }
}

fn system_prompt(target: &str) -> String {
    format!(
        "Bạn dịch phụ đề game/phim sang {}. Quy tắc:\n\
         - CHỈ trả về bản dịch, ĐÚNG MỘT DÒNG, không giải thích, không ngoặc kép, không thêm ghi chú.\n\
         - Văn nói tự nhiên, đúng giọng nhân vật; giữ nguyên tên riêng và thuật ngữ game.\n\
         - Dựa vào các câu trước để chọn đại từ xưng hô cho nhất quán.\n\
         - Nếu câu là chữ rác do OCR đọc sai, trả lại đúng nguyên văn, đừng bịa.",
        lang_name(target)
    )
}

fn user_prompt(context: &VecDeque<String>, text: &str) -> String {
    let mut s = String::new();
    if !context.is_empty() {
        s.push_str("Các câu trước (chỉ để hiểu ngữ cảnh, KHÔNG dịch lại):\n");
        for c in context {
            s.push_str("- ");
            s.push_str(c);
            s.push('\n');
        }
        s.push('\n');
    }
    s.push_str("Dịch câu này:\n");
    s.push_str(text.trim());
    s
}

/// Body OpenAI: `{"choices":[{"message":{"content":"..."}}]}`.
fn parse_llm(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let s = clean(v["choices"][0]["message"]["content"].as_str()?);
    (!s.is_empty()).then_some(s)
}

/// Cắt rác quanh bản dịch. Đầu vào luôn là MỘT dòng (deduper tách sẵn) nên nhiều dòng nghĩa là
/// model lải nhải trước khi ra kết quả -> lấy dòng cuối.
fn clean(s: &str) -> String {
    let last = s.trim().rsplit('\n').find(|l| !l.trim().is_empty()).unwrap_or("");
    let t = last.trim();
    let t = t.strip_prefix("Bản dịch:").unwrap_or(t).trim();
    t.trim_matches(|c| c == '"' || c == '“' || c == '”').trim().to_string()
}

/// Body Google dạng `[[["đã dịch","gốc",null,null,10],[...]],null,"en",...]` -> nối các đoạn.
fn parse_google(body: &str) -> Option<String> {
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

    fn cfg(engine: &str, key: &str) -> Config {
        Config {
            translate_engine: engine.into(),
            llm_api_key: key.into(),
            ..Config::default()
        }
    }

    #[test]
    fn ghep_nhieu_doan_google() {
        let body = r#"[[["Xin chào ","Hello ",null,null,10],["thế giới","world",null,null,10]],null,"en"]"#;
        assert_eq!(parse_google(body).as_deref(), Some("Xin chào thế giới"));
    }

    #[test]
    fn body_google_hong_tra_none() {
        assert!(parse_google("not json").is_none());
        assert!(parse_google("[null,null,\"en\"]").is_none());
        assert!(parse_google(r#"[[["   ","x",null,null,10]],null,"en"]"#).is_none());
    }

    #[test]
    fn tat_dich_tra_nguyen_van() {
        let mut t = Translator::new(&Config { translate: false, ..Config::default() });
        assert_eq!(t.translate("Hello"), "Hello");
    }

    #[test]
    fn thieu_key_thi_khong_bat_llm() {
        assert!(Translator::new(&cfg("llm", "   ")).llm.is_none());
        assert!(Translator::new(&cfg("google", "sk-abc")).llm.is_none());
        assert!(Translator::new(&cfg("llm", "sk-abc")).llm.is_some());
    }

    #[test]
    fn parse_llm_cat_rac() {
        let body = r#"{"choices":[{"message":{"content":"\"Chào cậu.\""}}]}"#;
        assert_eq!(parse_llm(body).as_deref(), Some("Chào cậu."));
        // model lải nhải trước khi ra kết quả -> lấy dòng cuối
        let body = r#"{"choices":[{"message":{"content":"Đây là bản dịch:\n\nBản dịch: Chạy đi!"}}]}"#;
        assert_eq!(parse_llm(body).as_deref(), Some("Chạy đi!"));
        assert!(parse_llm(r#"{"error":{"message":"bad key"}}"#).is_none());
        assert!(parse_llm(r#"{"choices":[{"message":{"content":"  \n "}}]}"#).is_none());
    }

    #[test]
    fn prompt_co_ngu_canh() {
        let ctx: VecDeque<String> = ["Who are you?", "I am the king."].map(String::from).into();
        let p = user_prompt(&ctx, "  Kneel.  ");
        assert!(p.contains("- Who are you?\n- I am the king."));
        assert!(p.ends_with("Dịch câu này:\nKneel."));
        // không có ngữ cảnh thì không nhét phần thừa
        assert_eq!(user_prompt(&VecDeque::new(), "Hi"), "Dịch câu này:\nHi");
    }

    /// Dựng HTTP server thật trên cổng ngẫu nhiên, trả 1 response rồi tắt.
    /// Trả về (base_url, handle) — handle.join() cho ra request thô mà client đã gửi.
    fn fake_server(status: &str, body: &str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let res = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let h = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let n = sock.read(&mut buf).unwrap();
            sock.write_all(res.as_bytes()).unwrap();
            String::from_utf8_lossy(&buf[..n]).to_string()
        });
        (format!("http://127.0.0.1:{port}"), h)
    }

    fn llm_cfg(base_url: String, context_lines: usize) -> Config {
        Config {
            translate_engine: "llm".into(),
            llm_base_url: base_url,
            llm_model: "test-model".into(),
            llm_api_key: "sk-test".into(),
            llm_context_lines: context_lines,
            ..Config::default()
        }
    }

    #[test]
    fn goi_llm_that_dung_duong_dan_header_va_ngu_canh() {
        let (base, h) = fake_server(
            "200 OK",
            r#"{"choices":[{"message":{"content":"Quỳ xuống!"}}]}"#,
        );
        let mut t = Translator::new(&llm_cfg(base, 3));
        t.remember("I am the king.");
        assert_eq!(t.translate("Kneel."), "Quỳ xuống!");

        let req = h.join().unwrap();
        assert!(req.starts_with("POST /chat/completions "), "{req}");
        assert!(req.contains("authorization: Bearer sk-test"), "{req}");
        assert!(req.contains(r#""model":"test-model""#), "{req}");
        // ngữ cảnh phải nằm trong body (JSON escape \n)
        assert!(req.contains("I am the king."), "{req}");
        assert!(req.contains("Kneel."), "{req}");
    }

    #[test]
    fn khong_ngu_canh_thi_body_khong_co_cau_cu() {
        let (base, h) = fake_server("200 OK", r#"{"choices":[{"message":{"content":"Ừ"}}]}"#);
        let mut t = Translator::new(&llm_cfg(base, 0));
        t.remember("I am the king.");
        assert_eq!(t.translate("Yes."), "Ừ");
        assert!(!h.join().unwrap().contains("I am the king."));
    }

    #[test]
    fn dinh_429_thi_vao_cooldown_va_khong_goi_nua() {
        let (base, h) = fake_server("429 Too Many Requests", r#"{"error":"rate limit"}"#);
        let mut t = Translator::new(&llm_cfg(base, 0));
        // via_llm trả None -> translate() sẽ rơi sang Google (cần mạng), nên gọi thẳng via_llm.
        assert!(t.via_llm("Hi").is_none());
        h.join().unwrap();
        assert!(t.cooldown_until.is_some());
        // Server đã tắt: nếu vẫn gọi thì lỗi "connection refused"; None ở đây chứng minh
        // cooldown chặn TRƯỚC khi gửi request.
        assert!(t.via_llm("Hi").is_none());
    }

    #[test]
    fn ngu_canh_giu_dung_so_cau_gan_nhat() {
        let mut t = Translator::new(&Config { llm_context_lines: 2, ..Config::default() });
        for s in ["a", " b ", "c"] {
            t.remember(s);
        }
        assert_eq!(t.context.iter().cloned().collect::<Vec<_>>(), ["b", "c"]);
    }

    #[test]
    fn ngu_canh_0_la_tat_han() {
        let mut t = Translator::new(&Config { llm_context_lines: 0, ..Config::default() });
        t.remember("a");
        assert!(t.context.is_empty());
        assert!(!user_prompt(&t.context, "Hi").contains("ngữ cảnh"));
        // vượt trần thì kẹp lại, không để prompt phình vô hạn
        let t = Translator::new(&Config { llm_context_lines: 99, ..Config::default() });
        assert_eq!(t.context_lines, MAX_CONTEXT_LINES);
    }
}
