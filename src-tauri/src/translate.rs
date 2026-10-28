//! Dịch câu nghe được sang tiếng Việt. Hai đường:
//!   - Google Translate (endpoint web `client=gtx`, không cần key) — nhanh; gửi kèm câu trước
//!     làm ngữ cảnh rồi chỉ lấy phần dịch của câu hiện tại, xem [`cat_phan_sau`].
//!   - LLM qua API **OpenAI-compatible** (`/chat/completions`): Groq, Gemini, OpenRouter,
//!     DeepSeek, OpenAI... chỉ khác `base_url` + `model` nên dùng chung một code path.
//! LLM được gửi kèm vài câu trước làm ngữ cảnh — đó là chỗ nó hơn Google Dịch (chọn đại từ
//! xưng hô nhất quán, hiểu thuật ngữ game).
//! Mọi lỗi đều lùi về Google, rồi cuối cùng là nguyên văn — không bao giờ chặn luồng đọc.

use std::collections::{HashMap, VecDeque};
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
/// Số câu đã dịch giữ lại. Game lặp thoại liên tục (NPC chào, câu hệ thống, combat bark).
const NHO_TOI_DA: usize = 500;
/// Câu trước cũ hơn mức này thì không gửi làm ngữ cảnh nữa — cảnh khác, người nói khác.
const NGU_CANH_GOOGLE: Duration = Duration::from_secs(30);

struct Llm {
    base_url: String,
    model: String,
    api_key: String,
}

pub struct Translator {
    /// Mã ngôn ngữ nguồn kiểu Google ("en", "ja", "ko", "zh-CN"). Đích luôn là tiếng Việt.
    source: String,
    llm: Option<Llm>,
    /// Câu gốc -> bản dịch. Có trong đây thì khỏi gọi mạng: nhanh hơn và đỡ bị Google chặn 429.
    nho: HashMap<String, String>,
    /// Câu gốc ngay trước + lúc nghe được, làm ngữ cảnh cho Google.
    truoc: Option<(String, Instant)>,
    /// Câu **gốc** (chưa dịch) gần nhất — LLM cần đọc mạch ngôn ngữ nguồn, không phải bản dịch.
    context: VecDeque<String>,
    /// 0 = tắt gửi ngữ cảnh (dịch từng câu rời, rẻ token và nhanh hơn).
    context_lines: usize,
    /// Tình trạng LLM hiện tại ("" = ổn). App là GUI, không ai đọc được stderr — key sai hay
    /// hết quota mà chỉ `eprintln!` thì người dùng thấy vẫn đọc bình thường (đã lùi về Google)
    /// và không hề biết AI đã ngừng chạy.
    notice: String,
    /// Thay đổi chưa đẩy lên UI. Chỉ set khi `notice` ĐỔI -> không spam mỗi câu.
    pending_notice: Option<String>,
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
            source: google_lang(&cfg.asr_language).into(),
            llm,
            nho: HashMap::new(),
            truoc: None,
            context: VecDeque::new(),
            context_lines: cfg.llm_context_lines.min(MAX_CONTEXT_LINES),
            notice: String::new(),
            pending_notice: None,
            cooldown_until: None,
            client,
        }
    }

    pub fn translate(&mut self, text: &str) -> String {
        let key = text.trim();
        if key.is_empty() {
            return text.to_string();
        }
        if let Some(o) = self.nho.get(key).cloned() {
            self.remember(key);
            return o;
        }
        let out = match self.via_llm(text).or_else(|| self.via_google(text)) {
            Some(o) => {
                // Google chạy lại được -> xoá cảnh báo "dịch lỗi". Khi LLM đang bật thì notice
                // là của LLM (đã tự quản lý), không đè lên.
                if self.llm.is_none() {
                    self.set_notice(String::new());
                }
                // shortcut: đầy thì xoá sạch thay vì bỏ câu cũ nhất (LRU); 500 câu là cả buổi chơi.
                if self.nho.len() >= NHO_TOI_DA {
                    self.nho.clear();
                }
                self.nho.insert(key.to_string(), o.clone());
                o
            }
            // Hỏng hết: vẫn trả nguyên văn để không nuốt câu, nhưng PHẢI báo — trước đây im lặng
            // nên người dùng chỉ thấy app đọc tiếng Anh bằng giọng Việt mà không biết vì sao.
            None => {
                self.set_notice("Dịch lỗi (không gọi được Google Dịch) — đang đọc nguyên văn".into());
                text.to_string()
            }
        };
        self.remember(key);
        out
    }

    /// Lấy thông báo cần hiện lên UI, nếu tình trạng vừa đổi. "" = vừa hoạt động lại bình thường.
    pub fn drain_notice(&mut self) -> Option<String> {
        self.pending_notice.take()
    }

    fn set_notice(&mut self, msg: String) {
        if self.notice != msg {
            self.notice = msg.clone();
            self.pending_notice = Some(msg);
        }
    }

    fn remember(&mut self, text: &str) {
        self.truoc = Some((text.trim().to_string(), Instant::now()));
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
                { "role": "system", "content": system_prompt(&self.source) },
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
                self.set_notice("AI hết quota (429) — tạm dùng Google Dịch 60s".into());
                return None;
            }
            Ok(r) if !r.status().is_success() => {
                let code = r.status().as_u16();
                let detail = r.text().ok().as_deref().and_then(api_error_message).unwrap_or_default();
                self.set_notice(format!("AI lỗi {code}{detail} — đang dùng Google Dịch"));
                return None;
            }
            Ok(r) => r.text().ok()?,
            Err(e) => {
                self.set_notice(format!("AI không gọi được ({e}) — đang dùng Google Dịch"));
                return None;
            }
        };

        let Some(out) = parse_llm(&body) else {
            self.set_notice("AI trả lời không đọc được — đang dùng Google Dịch".into());
            return None;
        };
        // Model nhỏ đôi khi bịa cả đoạn giải thích; dài bất thường thì thà lấy Google Dịch.
        if out.chars().count() > text.chars().count() * MAX_EXPANSION + 40 {
            self.set_notice("AI trả lời dài bất thường — câu này dùng Google Dịch".into());
            return None;
        }
        // Model hay trả lại nguyên câu gốc (nhất là câu ngắn) — đó là lỗi, không phải bản dịch.
        // Bỏ để Google dịch thay. (Google thì KHÔNG xét trùng: tên riêng, tiếng cười nó trả y
        // nguyên là đúng — xem `chua_dich`.)
        let trung = out.trim().eq_ignore_ascii_case(text.trim()) && text.chars().any(char::is_alphabetic);
        if trung || chua_dich(&out) {
            return None;
        }
        self.set_notice(String::new());
        Some(out)
    }

    /// Google gtx là endpoint không chính thức: thỉnh thoảng 429 / rớt mạng. Thử lại tối đa 3
    /// lần (nghỉ ngắn) trước khi bỏ cuộc; timeout thì không thử lại vì sẽ làm giọng trễ thêm.
    fn via_google(&self, text: &str) -> Option<String> {
        for lan in 0..3 {
            if lan > 0 {
                std::thread::sleep(Duration::from_millis(300 * lan));
            }
            match self.google_once(text) {
                Ok(out) => return Some(out),
                Err(retry) => {
                    if !retry {
                        break;
                    }
                }
            }
        }
        None
    }

    /// Err(true) = đáng thử lại, Err(false) = bỏ luôn.
    ///
    /// Gửi kèm câu trước (cách một dòng) để Google chọn nghĩa và xưng hô theo mạch thoại. Google
    /// trả bản dịch tách theo từng câu gốc nên cắt lấy đúng phần của câu hiện tại được.
    fn google_once(&self, text: &str) -> Result<String, bool> {
        let truoc = self.truoc.as_ref().filter(|(_, luc)| luc.elapsed() < NGU_CANH_GOOGLE);
        if let Some((p, _)) = truoc {
            // Mỗi vế phải đúng một dòng, không thì dấu xuống dòng ngăn cách mất tác dụng.
            let mot_dong = |s: &str| s.replace(['\r', '\n'], " ");
            let body = self.google_get(&format!("{}\n{}", mot_dong(p), mot_dong(text.trim())))?;
            if let Some(out) = cat_phan_sau(&body) {
                return if chua_dich(&out) { Err(true) } else { Ok(out) };
            }
            // Không tách được (Google gộp hai câu làm một đoạn) -> dịch riêng câu này.
        }
        let out = parse_google(&self.google_get(text)?).ok_or(true)?;
        if chua_dich(&out) {
            return Err(true);
        }
        Ok(out)
    }

    fn google_get(&self, q: &str) -> Result<String, bool> {
        let url = format!(
            "https://translate.googleapis.com/translate_a/single?client=gtx&sl={}&tl=vi&dt=t&q={}",
            self.source,
            urlencoding::encode(q)
        );
        let res = self.client.get(&url).send().map_err(|e| {
            eprintln!("[translate] Google lỗi: {e}");
            !e.is_timeout()
        })?;
        if !res.status().is_success() {
            eprintln!("[translate] Google trả {}", res.status());
            return Err(true);
        }
        res.text().map_err(|_| true)
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
    if !llm_enabled(cfg) {
        return "dịch Google".into();
    }
    match cfg.llm_context_lines.min(MAX_CONTEXT_LINES) {
        0 => format!("dịch AI ({}, không ngữ cảnh)", cfg.llm_model),
        n => format!("dịch AI ({}, ngữ cảnh {n} câu)", cfg.llm_model),
    }
}

/// Mã ngôn ngữ của app (cũng là mã Speechmatics) -> mã Google. Google cần "zh-CN", không nhận "zh".
pub fn google_lang(code: &str) -> &str {
    match code {
        "zh" => "zh-CN",
        c => c,
    }
}

pub fn ten_tieng(code: &str) -> &str {
    match code {
        "ja" => "tiếng Nhật",
        "ko" => "tiếng Hàn",
        c if c.starts_with("zh") => "tiếng Trung",
        _ => "tiếng Anh",
    }
}

fn system_prompt(source: &str) -> String {
    format!(
        "Bạn dịch lời thoại game/phim từ {} sang tiếng Việt. Quy tắc:\n\
         - CHỈ trả về bản dịch, ĐÚNG MỘT DÒNG, không giải thích, không ngoặc kép, không thêm ghi chú.\n\
         - Văn nói tự nhiên, đúng giọng nhân vật; giữ nguyên tên riêng và thuật ngữ game.\n\
         - Dựa vào các câu trước để chọn đại từ xưng hô cho nhất quán.\n\
         - Câu do máy nghe giọng nói chép lại nên có thể sai vài từ: đoán theo ngữ cảnh, đừng bịa thêm.",
        ten_tieng(source)
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

/// Rút câu lỗi từ body `{"error":{"message":"..."}}` (mọi provider OpenAI-compatible đều dùng
/// dạng này). Không parse được thì trả None — thà báo mỗi mã HTTP còn hơn dội JSON thô lên UI.
fn api_error_message(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let msg = v["error"]["message"].as_str()?;
    Some(format!(": {}", msg.chars().take(120).collect::<String>()))
}

/// Body OpenAI: `{"choices":[{"message":{"content":"..."}}]}`.
fn parse_llm(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let s = clean(v["choices"][0]["message"]["content"].as_str()?);
    (!s.is_empty()).then_some(s)
}

/// Cắt rác quanh bản dịch. Đầu vào luôn là MỘT câu (khâu nghe tách sẵn) nên nhiều dòng nghĩa là
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

/// Bản "dịch" vẫn còn hơn 1/3 số chữ là chữ Nhật/Hàn/Trung, tức Google trả lại nguyên văn (bắt
/// được 10/2026: cùng một câu tiếng Nhật, một lượt ra "5十円の学校販売のパンを買う.", các lượt
/// khác dịch đúng) — thử lại thường là được.
///
/// KHÔNG xét "trùng câu gốc": bản trước có xét, và tên riêng ("Shepard."), tiếng cười ("Ha ha
/// ha.") — Google trả y nguyên là ĐÚNG — bị thử lại 3 lần, chậm thêm ~1,4s rồi còn báo lỗi oan.
fn chua_dich(dich: &str) -> bool {
    let cjk = |c: char| {
        matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0x1100..=0x11FF
            | 0x3130..=0x318F | 0xAC00..=0xD7AF)
    };
    let chu = dich.chars().filter(|c| c.is_alphabetic()).count();
    dich.chars().filter(|&c| cjk(c)).count() * 3 > chu
}

/// Lấy phần dịch của câu hiện tại từ body của lượt gửi "câu trước\ncâu này". Google giữ nguyên
/// dấu xuống dòng trong bản dịch, nhưng tách đoạn mỗi thứ tiếng một kiểu (đo 10/2026): tiếng Anh
/// ra hai đoạn, còn Nhật/Hàn/Trung dịch vòng qua tiếng Anh nên ra MỘT đoạn chứa "\n" — vì vậy cắt
/// theo dấu xuống dòng chứ không theo đoạn. Không có dấu đó (bị gộp) -> None.
fn cat_phan_sau(body: &str) -> Option<String> {
    let tat_ca = parse_google(body)?;
    let (_, sau) = tat_ca.split_once('\n')?;
    let sau = sau.trim();
    (!sau.is_empty()).then(|| sau.to_string())
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
    fn google_chi_lay_phan_cua_cau_hien_tai() {
        // Body thật (rút gọn) Google trả 10/2026. Tiếng Anh: hai đoạn.
        let en = r#"[[["Cô ấy đã rời bỏ tôi.\n","She left me.\n",null,null,3],["Tôi sẽ tìm cô ấy.","I will find her.",null,null,3]],null,"en"]"#;
        assert_eq!(cat_phan_sau(en).as_deref(), Some("Tôi sẽ tìm cô ấy."));
        // Tiếng Nhật: MỘT đoạn chứa "\n" — cắt theo đoạn là hỏng ở đây.
        let ja = r#"[[["Bạn là ai?\nTôi là anh trai của bạn.","お前は誰だ？\n俺はお前の兄だ。",null,null,3]],null,"ja"]"#;
        assert_eq!(cat_phan_sau(ja).as_deref(), Some("Tôi là anh trai của bạn."));
        // Mất dấu xuống dòng (gộp làm một câu) -> None để dịch riêng câu này.
        let gop = r#"[[["Ta là vua, quỳ xuống!","I am the king. Kneel!",null,null,3]],null,"en"]"#;
        assert!(cat_phan_sau(gop).is_none());
    }

    #[test]
    fn nhan_ra_ban_dich_van_la_nguyen_van() {
        assert!(chua_dich("5十円の学校販売のパンを買う."));
        assert!(chua_dich("집에 가요."));
        assert!(!chua_dich("Quỳ xuống!"));
        // Google trả y nguyên tên riêng / tiếng cười là đúng — đo thật 10/2026, không được thử lại.
        assert!(!chua_dich("Shepard."));
        assert!(!chua_dich("Ha ha ha."));
        // Bản dịch đúng mà còn giữ một tên riêng chữ Hán thì KHÔNG được coi là lỗi.
        assert!(!chua_dich("Tôi sẽ đến chùa 金閣寺 vào ngày mai cùng với bạn bè của tôi."));
    }

    #[test]
    fn cau_da_dich_khong_goi_mang_lai() {
        let mut t = Translator::new(&Config::default());
        t.nho.insert("Hello".into(), "Xin chào".into());
        // Không có mạng giả nào ở đây: gọi Google thật sẽ ra khác hoặc lỗi. Trùng là nhờ bộ nhớ.
        assert_eq!(t.translate("  Hello "), "Xin chào");
        // Câu vừa lấy từ bộ nhớ vẫn phải thành ngữ cảnh cho câu sau.
        assert_eq!(t.truoc.as_ref().map(|(s, _)| s.as_str()), Some("Hello"));
    }

    #[test]
    fn ngon_ngu_nguon_theo_tieng_trong_game() {
        for (vao, ra) in [("en", "en"), ("ja", "ja"), ("ko", "ko"), ("zh", "zh-CN")] {
            let t = Translator::new(&Config { asr_language: vao.into(), ..Config::default() });
            assert_eq!(t.source, ra);
        }
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
    fn loi_llm_bao_len_ui_va_tu_xoa_khi_chay_lai() {
        // key sai -> phải có thông báo kèm câu lỗi thật của provider
        let (base, h) = fake_server(
            "401 Unauthorized",
            r#"{"error":{"message":"Invalid API Key"}}"#,
        );
        let mut t = Translator::new(&llm_cfg(base, 0));
        assert!(t.via_llm("Hi").is_none());
        h.join().unwrap();
        let notice = t.drain_notice().expect("phải có thông báo");
        assert!(notice.contains("401") && notice.contains("Invalid API Key"), "{notice}");
        // chỉ báo MỘT lần cho tới khi tình trạng đổi -> không spam mỗi câu
        assert!(t.drain_notice().is_none());

        // gọi lại thành công -> "" để UI xoá cảnh báo
        let (base2, h2) = fake_server("200 OK", r#"{"choices":[{"message":{"content":"Chào"}}]}"#);
        t.llm.as_mut().unwrap().base_url = base2;
        assert_eq!(t.via_llm("Hi").as_deref(), Some("Chào"));
        h2.join().unwrap();
        assert_eq!(t.drain_notice().as_deref(), Some(""));
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
    fn llm_tra_lai_nguyen_cau_tieng_anh_thi_bo() {
        let (base, h) = fake_server("200 OK", r#"{"choices":[{"message":{"content":"kneel."}}]}"#);
        let mut t = Translator::new(&llm_cfg(base, 0));
        assert!(t.via_llm("Kneel.").is_none());
        h.join().unwrap();
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
