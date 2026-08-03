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
    pub dedup_require_stable: bool,
    pub min_line_len: usize,
    pub fuzzy_threshold: f64,

    /// "en" (Anh->Việt) | "zh" (Trung->Việt). Bản Rust chưa có OCR tiếng Việt nên bỏ "plain".
    pub mode: String,

    pub ocr_language: String,
    /// Bỏ dòng OCR thấp hơn `tỉ lệ × dòng cao nhất` — cắt chữ nhỏ của giao diện (thanh tua,
    /// tiêu đề video) lọt vào khung quét. 0 = tắt. Chỉnh xuống nếu phụ đề bị bỏ sót.
    pub min_height_ratio: f32,
    /// Ngưỡng nhị phân hoá cho lượt OCR thứ hai (max(R,G,B) >= ngưỡng coi là chữ).
    /// Hạ xuống nếu phụ đề mờ/xám, nâng lên nếu nền sáng bị nhận nhầm thành chữ.
    pub binarize_threshold: u8,
    pub translate: bool,
    pub translate_from: String,
    pub translate_to: String,

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
            interval_ms: 1200,
            dedup_require_stable: true,
            min_line_len: 2,
            fuzzy_threshold: 0.82,
            mode: "en".into(),
            ocr_language: "en-US".into(),
            min_height_ratio: 0.6,
            binarize_threshold: 150,
            translate: true,
            translate_from: "en".into(),
            translate_to: "vi".into(),
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
        assert_eq!(c.interval_ms, 1200);
    }
}
