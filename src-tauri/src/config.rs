//! Đọc/ghi cấu hình ViNarrator. Giữ nguyên tên field (snake_case) nên config.json cũ đọc được —
//! field của bản có OCR (region, mode, source…) bị serde bỏ qua. File nằm ở
//! %APPDATA%/ViNarrator/config.json.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Nghe bằng gì: "local" (model chạy máy, không cần mạng) |
    /// "speechmatics" (đám mây, cần key, chữ trả về có sẵn dấu câu).
    pub asr_engine: String,
    /// Key Speechmatics — để trong config của user (`%APPDATA%`), KHÔNG hardcode.
    pub speechmatics_key: String,
    /// Tiếng trong game: "en" | "ja" | "ko" | "zh". Quyết định cả model nghe chạy máy (xem
    /// [`crate::asr`]) lẫn ngôn ngữ nguồn gửi đi dịch. Đích luôn là tiếng Việt.
    pub asr_language: String,

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
    /// 0..300 phần trăm. Trên 100 là khuếch đại thật (có limiter), xem
    /// [`crate::runtime::apply_gain`] — cần thiết vì giọng đọc chìm nghỉm bên cạnh tiếng game
    /// vốn đã được nén tối đa.
    pub volume: u16,
    /// Phụ đề nổi trong game: cửa sổ trong suốt luôn nằm trên cùng.
    pub captions: bool,
    /// false = chuột xuyên qua (không cản việc chơi game); true = kéo được để dời vị trí.
    pub captions_movable: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Mặc định chạy máy: không cần key, không gửi lời thoại đi đâu.
            asr_engine: "local".into(),
            speechmatics_key: String::new(),
            asr_language: "en".into(),
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
            captions: false,
            captions_movable: false,
        }
    }
}

pub fn path() -> PathBuf {
    let dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    dir.join("ViNarrator").join("config.json")
}

pub fn load() -> Config {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str::<Config>(&s).ok())
        .unwrap_or_default()
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
    fn config_json_cu_van_doc_duoc() {
        // field của bản có OCR (region, mode, source, ocr_engine…) bị bỏ qua, field thiếu lấy
        // default — config cũ KHÔNG được làm app mất luôn cài đặt khác của người dùng.
        let s = r#"{"region":{"left":1,"top":2,"width":3,"height":4},"mode":"zh",
                    "source":"screen","ocr_engine":"oneocr","volume":40,"asr_language":"ja"}"#;
        let c: Config = serde_json::from_str(s).unwrap();
        assert_eq!(c.volume, 40);
        assert_eq!(c.asr_language, "ja");
        // config có trước khi thêm engine neural -> phải về giọng Windows, KHÔNG được rơi vào
        // chuỗi rỗng rồi đọc câm.
        assert_eq!(c.tts_engine, "windows");
        assert_eq!(c.asr_engine, "local");
    }
}
