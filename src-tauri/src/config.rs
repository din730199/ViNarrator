//! Đọc/ghi cấu hình ViNarrator. Port từ config.py — giữ nguyên tên field (snake_case)
//! nên config.json cũ đọc được. File nằm ở %APPDATA%/ViNarrator/config.json.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub region: Region,
    /// Vùng bỏ qua trong khung quét (toạ độ màn hình), None = không loại trừ.
    pub exclude_region: Option<Region>,
    pub interval_ms: u64,

    // Chống đọc trùng phụ đề cuộn
    /// Chỉ đọc dòng đã thấy ở khung TRƯỚC -> không đọc nửa câu đang hiện dần.
    /// Cái giá: phụ đề phải đứng yên **1–2 lần `interval_ms`** mới được đọc, chớp nhanh hơn
    /// là mất trắng. Đó là lý do `interval_ms` phải nhỏ (xem mặc định).
    pub dedup_require_stable: bool,
    pub min_line_len: usize,
    /// Hai dòng giống nhau tới mức này (0..1) thì coi là trùng, không đọc lại.
    /// **Đo được** trên 60 ảnh phụ đề xuống cấp: xem `LEGACY_FUZZY` và README.
    pub fuzzy_threshold: f64,

    /// "en" (Anh->Việt) | "zh" (Trung->Việt). Bản Rust chưa có OCR tiếng Việt nên bỏ "plain".
    pub mode: String,

    /// Lấy lời thoại từ đâu: "screen" (OCR vùng phụ đề) | "audio" (nghe loopback rồi nhận
    /// dạng tại chỗ). "audio" CHỈ nghe được tiếng Anh — chế độ zh vẫn phải dùng "screen".
    /// Chưa tải model (~41 MB) thì tự chạy như "screen", không câm.
    pub source: String,

    /// Nghe bằng gì: "local" (model chạy máy, chỉ tiếng Anh, không cần mạng) |
    /// "speechmatics" (đám mây, cần key, nhiều thứ tiếng, chữ trả về có sẵn dấu câu).
    pub asr_engine: String,
    /// Key Speechmatics — để trong config của user (`%APPDATA%`), KHÔNG hardcode.
    pub speechmatics_key: String,
    /// Mã ngôn ngữ Speechmatics ("en", "zh", "ja"…). Bản chạy máy bỏ qua field này.
    pub asr_language: String,

    /// "oneocr" (mặc định, chính xác hơn — cần Win11 + Snipping Tool) | "windows"
    /// (Windows.Media.Ocr, nhẹ & nhanh hơn, chạy từ Win10).
    pub ocr_engine: String,
    pub ocr_language: String,
    /// Bỏ dòng OCR thấp hơn `tỉ lệ × dòng cao nhất` — cắt chữ nhỏ của giao diện (thanh tua,
    /// tiêu đề video) lọt vào khung quét. 0 = tắt. Chỉnh xuống nếu phụ đề bị bỏ sót.
    pub min_height_ratio: f32,
    /// Ngưỡng nhị phân hoá cho lượt OCR thứ hai (max(R,G,B) >= ngưỡng coi là chữ).
    /// Hạ xuống nếu phụ đề mờ/xám, nâng lên nếu nền sáng bị nhận nhầm thành chữ.
    pub binarize_threshold: u8,
    /// Bỏ dòng OCR có điểm "sạch" chuẩn hoá dưới ngưỡng này (0..1) — chặn chữ bịa.
    /// Nâng lên nếu vẫn nghe đọc chữ rác; hạ xuống (hoặc 0 để tắt) nếu câu thật bị bỏ oan.
    pub min_clean_ratio: f32,
    pub translate: bool,
    pub translate_from: String,
    pub translate_to: String,

    /// "google" (endpoint web, không cần key) | "llm" (API OpenAI-compatible).
    /// Để "llm" mà `llm_api_key` rỗng thì vẫn chạy Google — xem `translate.rs`.
    pub translate_engine: String,
    /// Base URL kiểu OpenAI (không kèm `/chat/completions`). Mặc định Groq: free tier
    /// 30 lệnh/phút · 14.400/ngày, đủ cho cả buổi chơi, lại là provider nhanh nhất.
    pub llm_base_url: String,
    pub llm_model: String,
    /// Key để trong config của user (`%APPDATA%`), KHÔNG hardcode và không đóng gói kèm.
    pub llm_api_key: String,
    /// Số câu trước gửi kèm cho LLM để chọn xưng hô nhất quán. 0 = tắt (rẻ token, nhanh hơn).
    pub llm_context_lines: usize,

    /// "windows" (Windows.Media.SpeechSynthesis, có sẵn) |
    /// "vieneu" (neural 48 kHz đọc lẫn Anh–Việt, ~560 MB + 1,3 GB RAM).
    /// Chọn "vieneu" mà chưa cài xong thì vẫn đọc bằng giọng Windows, không câm.
    pub tts_engine: String,
    /// Tên giọng WinRT (engine "windows") hoặc một trong 14 giọng của VieNeu.
    pub voice_name: String,
    /// bội số 0.5..2.0
    pub speed: f64,
    /// 0..100
    pub volume: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            region: Region { left: 100, top: 100, width: 800, height: 200 },
            exclude_region: None,
            // 700 chứ không phải 1200 của bản Python: nhân với `dedup_require_stable` thì 1200
            // bắt phụ đề phải hiện 1,2-2,4s mới đọc được — chớp nhanh hơn là mất. Đo trên máy
            // thật: OneOCR 73ms/khung, Windows OCR 16ms -> ở nhịp 700ms chỉ tốn ~10% một nhân.
            interval_ms: 700,
            dedup_require_stable: true,
            // 3, không phải 4: rác dấu câu (".", "-", "|", "•") đã bị `min_clean_ratio` dọn,
            // phần còn lại chỉ là mảnh chữ 1-2 ký tự ("o", "ll", "at"). Lên 4 sẽ nuốt luôn
            // thoại thật đúng 3 ký tự: "Yes", "No!", "Go!", "Run".
            min_line_len: 3,
            fuzzy_threshold: 0.97,
            mode: "en".into(),
            // Mặc định đọc màn hình: không phải tải gì, và phụ đề game thường chính xác hơn
            // nhận dạng tiếng giữa nhạc nền + tiếng nổ.
            source: "screen".into(),
            // Mặc định chạy máy: không cần key, không gửi lời thoại đi đâu.
            asr_engine: "local".into(),
            speechmatics_key: String::new(),
            asr_language: "en".into(),
            ocr_engine: "oneocr".into(),
            ocr_language: "en-US".into(),
            min_height_ratio: 0.6,
            binarize_threshold: 150,
            min_clean_ratio: 0.6,
            translate: true,
            translate_from: "en".into(),
            translate_to: "vi".into(),
            translate_engine: "google".into(),
            llm_base_url: "https://api.groq.com/openai/v1".into(),
            llm_model: "llama-3.3-70b-versatile".into(),
            llm_api_key: String::new(),
            llm_context_lines: 3,
            // Mặc định giọng có sẵn: cài xong là chạy được ngay, không bắt tải 560 MB mới
            // nghe được câu đầu. Ai muốn giọng tự nhiên hơn thì tự bật VieNeu trên UI.
            tts_engine: "windows".into(),
            voice_name: "Microsoft An".into(),
            speed: 1.0,
            volume: 100,
        }
    }
}

impl Config {
    /// Áp chế độ cấp cao -> set ngôn ngữ OCR + dịch tương ứng (port apply_mode của config.py).
    pub fn apply_mode(&mut self, mode: &str) {
        match mode {
            "zh" => {
                self.mode = "zh".into();
                self.ocr_language = "zh-Hans-CN".into();
                self.translate = true;
                self.translate_from = "zh-CN".into();
                self.translate_to = "vi".into();
            }
            // "en" và mọi giá trị lạ (kể cả "translated"/"plain" của config cũ) -> Anh->Việt.
            _ => {
                self.mode = "en".into();
                self.ocr_language = "en-US".into();
                self.translate = true;
                self.translate_from = "en".into();
                self.translate_to = "vi".into();
            }
        }
    }
}

/// Ngưỡng cũ, thừa hưởng từ bản Python (`difflib.SequenceMatcher`) rồi bê nguyên sang bản Rust
/// (`normalized_levenshtein`) mà chưa hề đo. Đo lại trên 60 ảnh phụ đề xuống cấp thì nó **nuốt
/// 8/8 cặp thoại chỉ khác một từ** ("...touch that lever" vs "...that button" = 0.829), tức là
/// mất hẳn câu thoại. Đổi lấy 0 lần đọc lặp — một cái giá quá đắt.
/// Chỉ nâng đúng giá trị mặc định cũ này: ai tự chỉnh số khác thì giữ nguyên.
const LEGACY_FUZZY: f64 = 0.82;

fn migrate(c: &mut Config) {
    if c.fuzzy_threshold == LEGACY_FUZZY {
        c.fuzzy_threshold = Config::default().fuzzy_threshold;
    }
}

pub fn path() -> PathBuf {
    let dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    dir.join("ViNarrator").join("config.json")
}

pub fn load() -> Config {
    let p = path();
    match std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str::<Config>(&s).ok()) {
        Some(mut c) => {
            // config.json cũ có mode "translated"/"plain" -> chuẩn hoá về en/zh.
            let m = c.mode.clone();
            c.apply_mode(&m);
            migrate(&mut c);
            c
        }
        None => Config::default(),
    }
}

pub fn save(cfg: &Config) -> Result<(), String> {
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let s = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&p, s).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_cu_duoc_chuan_hoa() {
        let mut c = Config::default();
        c.apply_mode("translated");
        assert_eq!(c.mode, "en");
        assert_eq!(c.ocr_language, "en-US");
        c.apply_mode("zh");
        assert_eq!(c.translate_from, "zh-CN");
        // "plain" không còn hỗ trợ -> quay về en, KHÔNG để lại ocr_language tiếng Trung.
        c.apply_mode("plain");
        assert_eq!(c.mode, "en");
        assert_eq!(c.ocr_language, "en-US");
    }

    #[test]
    fn config_json_cu_van_doc_duoc() {
        // field thừa (ocr_backend, easyocr_*) bị bỏ qua, field thiếu lấy default.
        let s = r#"{"region":{"left":1,"top":2,"width":3,"height":4},
                    "ocr_backend":"easyocr","easyocr_languages":["vi"],"volume":40}"#;
        let c: Config = serde_json::from_str(s).unwrap();
        assert_eq!(c.region.width, 3);
        assert_eq!(c.volume, 40);
        assert_eq!(c.interval_ms, Config::default().interval_ms);
        assert_eq!(c.min_clean_ratio, 0.6);
        // config có trước khi thêm engine neural -> phải về giọng Windows, KHÔNG được rơi vào
        // chuỗi rỗng rồi đọc câm (tts.rs so bằng == "vieneu" nên chuỗi lạ vẫn an toàn, nhưng
        // UI sẽ hiện dropdown trống).
        assert_eq!(c.tts_engine, "windows");
        // Config có trước khi thêm nguồn âm thanh -> phải về "screen". Rơi vào chuỗi rỗng thì
        // narrator không dựng OCR mà cũng không dựng ASR -> app chạy nhưng câm tịt.
        assert_eq!(c.source, "screen");
    }

    #[test]
    fn nguong_fuzzy_cu_duoc_nang_len() {
        // đúng giá trị mặc định cũ -> nâng
        let mut c = Config { fuzzy_threshold: LEGACY_FUZZY, ..Config::default() };
        migrate(&mut c);
        assert_eq!(c.fuzzy_threshold, Config::default().fuzzy_threshold);
        // người dùng tự chỉnh số khác -> KHÔNG đụng vào
        let mut c = Config { fuzzy_threshold: 0.7, ..Config::default() };
        migrate(&mut c);
        assert_eq!(c.fuzzy_threshold, 0.7);
    }
}

