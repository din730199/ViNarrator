//! Hai engine OCR, chọn được trong giao diện:
//!
//! - **OneOCR** (mặc định) — engine của Snipping Tool Win11, xem [`crate::oneocr`].
//! - **Windows OCR** — `Windows.Media.Ocr`, có sẵn mọi máy Win10+, nhẹ và nhanh hơn.
//!
//! Cả hai đều KHÔNG đọc tốt tiếng Việt (đo được 0.88 và 0.70) nên chỉ dùng cho chế độ dịch
//! Anh/Trung -> Việt. Ngôn ngữ chưa cài gói -> Windows OCR lùi về ngôn ngữ hồ sơ người dùng,
//! `info()` báo lại tag thật.
//!
//! Hai cải tiến của nhánh Windows OCR (đo trên 8 mẫu, xem `cleanliness`):
//!  1. Lọc theo CHIỀU CAO dòng — bỏ chữ nhỏ của giao diện (thanh tua, tiêu đề video) lọt vào
//!     khung quét. Phụ đề h≈31px, chữ UI h≈9px -> tỉ lệ 0.29, ngưỡng 0.6 tách sạch. OneOCR
//!     cũng lọc y hệt: confidence KHÔNG dùng được vì chữ rác cũng được đọc đúng (~0.99).
//!  2. OCR HAI LƯỢT (ảnh gốc + ảnh nhị phân hoá) rồi chọn bản ít ký tự rác hơn. Chữ màu trên
//!     nền ảnh động (phụ đề video) ảnh gốc đọc sai nặng; nhị phân hoá đọc đúng. Ngược lại chữ
//!     đen trên nền sáng thì nhị phân hoá phá ảnh — nên phải CHỌN chứ không thay thế.
//!     OneOCR không cần trò này (tự nó đã 0.99).

use windows::core::HSTRING;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;

use crate::config::Config;
use crate::oneocr::OneOcr;

pub enum Ocr {
    One(OneOcr),
    Win(WindowsOcr),
}

impl Ocr {
    /// Dựng engine theo cấu hình. Chọn OneOCR mà máy không có Snipping Tool -> tự lùi về
    /// Windows OCR và kèm lý do trong `info()` (hiện trên thanh trạng thái).
    pub fn new(cfg: &Config) -> Result<Self, String> {
        if cfg.ocr_engine != "windows" {
            match OneOcr::new() {
                Ok(o) => return Ok(Ocr::One(o)),
                Err(e) => {
                    let w = WindowsOcr::new(&cfg.ocr_language)?;
                    return Ok(Ocr::Win(WindowsOcr { fallback_reason: Some(e), ..w }));
                }
            }
        }
        Ok(Ocr::Win(WindowsOcr::new(&cfg.ocr_language)?))
    }

    pub fn info(&self) -> String {
        match self {
            Ocr::One(o) => o.info(),
            Ocr::Win(w) => w.info(),
        }
    }

    /// rgba: dữ liệu thô từ [`crate::capture`] (RGBA). Nhánh Windows tự đảo sang BGRA.
    pub fn recognize_rgba(&self, rgba: &[u8], width: u32, height: u32, cfg: &Config) -> Result<String, String> {
        let latin = is_latin_mode(cfg);
        let text = match self {
            Ocr::One(o) => o.recognize_rgba(rgba, width, height, cfg.min_height_ratio)?,
            Ocr::Win(w) => {
                let mut bgra = rgba.to_vec();
                for px in bgra.chunks_exact_mut(4) {
                    px.swap(0, 2);
                }
                w.recognize_bgra(
                    &bgra,
                    width as i32,
                    height as i32,
                    latin,
                    cfg.min_height_ratio,
                    cfg.binarize_threshold,
                )?
            }
        };
        Ok(drop_junk_lines(&text, latin, cfg.min_clean_ratio))
    }
}

/// Chế độ nguồn dùng chữ Latin? "en" có, "zh" không — quyết định việc phạt chữ ngoài ASCII.
fn is_latin_mode(cfg: &Config) -> bool {
    cfg.mode != "zh"
}

/// Bỏ những dòng OCR bịa. Áp cho CẢ HAI engine — trước đây `cleanliness` chỉ dùng để chọn
/// giữa hai lượt của Windows OCR, chưa bao giờ loại dòng, nên rác đi thẳng vào TTS.
/// `min_ratio <= 0` = tắt lọc.
pub fn drop_junk_lines(text: &str, latin: bool, min_ratio: f32) -> String {
    if min_ratio <= 0.0 {
        return text.to_string();
    }
    text.lines()
        .filter(|l| l.trim().is_empty() || clean_ratio(l, latin) >= min_ratio)
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct WindowsOcr {
    engine: OcrEngine,
    requested: String,
    /// Có giá trị khi engine này được dùng vì OneOCR không khả dụng — hiện lý do cho người dùng.
    fallback_reason: Option<String>,
}

impl WindowsOcr {
    pub fn new(language_tag: &str) -> Result<Self, String> {
        let engine = Language::CreateLanguage(&HSTRING::from(language_tag))
            .and_then(|l| OcrEngine::TryCreateFromLanguage(&l))
            .or_else(|_| OcrEngine::TryCreateFromUserProfileLanguages())
            .map_err(|e| format!("không tạo được OCR engine: {e}"))?;
        Ok(Self { engine, requested: language_tag.to_string(), fallback_reason: None })
    }

    /// Tag ngôn ngữ engine thực sự dùng (có thể khác cái yêu cầu nếu chưa cài gói).
    pub fn actual_language(&self) -> String {
        self.engine
            .RecognizerLanguage()
            .and_then(|l| l.LanguageTag())
            .map(|t| t.to_string())
            .unwrap_or_default()
    }

    pub fn info(&self) -> String {
        let actual = self.actual_language();
        let base = if actual.eq_ignore_ascii_case(&self.requested) {
            format!("windows({actual})")
        } else {
            format!("windows({actual} — CHƯA CÀI gói {})", self.requested)
        };
        match &self.fallback_reason {
            Some(r) => format!("{base} — không dùng được OneOCR: {r}"),
            None => base,
        }
    }

    pub fn recognize_bgra(
        &self,
        bgra: &[u8],
        width: i32,
        height: i32,
        latin: bool,
        min_height_ratio: f32,
        binarize_threshold: u8,
    ) -> Result<String, String> {
        if bgra.len() != (width as usize) * (height as usize) * 4 {
            return Ok(String::new());
        }
        let raw = self.pass(bgra, width, height, min_height_ratio)?;
        let bin = self.pass(&binarize(bgra, binarize_threshold), width, height, min_height_ratio)?;
        Ok(if cleanliness(&bin, latin) > cleanliness(&raw, latin) { bin } else { raw })
    }

    fn pass(&self, bgra: &[u8], width: i32, height: i32, min_ratio: f32) -> Result<String, String> {
        let run = || -> windows::core::Result<String> {
            let buf = CryptographicBuffer::CreateFromByteArray(bgra)?;
            let bmp = SoftwareBitmap::CreateCopyFromBuffer(&buf, BitmapPixelFormat::Bgra8, width, height)?;
            let result = self.engine.RecognizeAsync(&bmp)?.join()?;

            let mut lines: Vec<(f32, String)> = Vec::new();
            for line in result.Lines()? {
                // Chiều cao dòng = TRUNG VỊ chiều cao các từ — dấu câu/chữ thường lẻ không kéo lệch.
                let mut hs: Vec<f32> = Vec::new();
                for w in line.Words()? {
                    hs.push(w.BoundingRect()?.Height);
                }
                hs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let h = hs.get(hs.len() / 2).copied().unwrap_or(0.0);
                lines.push((h, line.Text()?.to_string()));
            }
            let tallest = lines.iter().map(|l| l.0).fold(0.0f32, f32::max);
            Ok(lines
                .iter()
                .filter(|l| min_ratio <= 0.0 || tallest <= 0.0 || l.0 / tallest >= min_ratio)
                .map(|l| l.1.as_str())
                .collect::<Vec<_>>()
                .join("\n"))
        };
        run().map_err(|e| e.to_string())
    }
}

/// max(R,G,B) >= ngưỡng -> đen, còn lại -> trắng. Chữ sáng (trắng/vàng/cyan) tách khỏi nền ảnh,
/// và thành chữ ĐEN trên nền TRẮNG — đúng thứ Windows OCR đọc tốt nhất.
fn binarize(bgra: &[u8], threshold: u8) -> Vec<u8> {
    let mut out = vec![255u8; bgra.len()];
    for (i, px) in bgra.chunks_exact(4).enumerate() {
        if px[0].max(px[1]).max(px[2]) >= threshold {
            out[i * 4..i * 4 + 3].fill(0);
        }
    }
    out
}

/// Dấu câu thường: KHÔNG tính vào cả tử lẫn mẫu, nếu không "Hmm..." (3/6) sẽ bị coi là rác.
const NEUTRAL: &str = ",.!?'\"-:;…";
/// Chữ có dấu lọt vào khung tiếng Anh gần như chắc chắn là OCR bịa (`Cả`, `Phả` trong
/// `' oanqCảaTôi Phả Server Minecraft`). Phạt nặng hơn ký tự lạ vì tín hiệu mạnh hơn.
const FOREIGN_LETTER: i32 = -6;
const ODD_CHAR: i32 = -3;

/// None = dấu câu trung tính (bỏ khỏi phép tính).
fn char_score(c: char, latin: bool) -> Option<i32> {
    if NEUTRAL.contains(c) {
        return None;
    }
    Some(if c.is_alphanumeric() || c.is_whitespace() {
        if latin && c.is_alphabetic() && !c.is_ascii() {
            FOREIGN_LETTER
        } else {
            1
        }
    } else {
        ODD_CHAR
    })
}

/// Điểm "sạch" TUYỆT ĐỐI — dùng để chọn giữa hai lượt Windows OCR (so hai chuỗi cùng nội dung).
/// Ảnh bị nhị phân hoá sai làm OCR bấm nát chữ -> sinh nhiều ký tự lạ -> điểm thấp.
fn cleanliness(s: &str, latin: bool) -> i32 {
    s.chars().filter_map(|c| char_score(c, latin)).sum()
}

/// Điểm "sạch" CHUẨN HOÁ 0..1 — dùng để loại dòng rác, không phụ thuộc độ dài dòng.
/// Dòng đọc đúng ≈ 1.0; `' oanqCảaTôi Phả Server Minecraft Bi Bo H` ≈ 0.50.
fn clean_ratio(s: &str, latin: bool) -> f32 {
    let scores: Vec<i32> = s.chars().filter_map(|c| char_score(c, latin)).collect();
    if scores.is_empty() {
        return 0.0;
    }
    scores.iter().sum::<i32>() as f32 / scores.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nhi_phan_hoa_theo_kenh_sang_nhat() {
        // 2 pixel BGRA: cyan (B=238,G=211,R=34) và nâu tối (B=40,G=60,R=70)
        let src = [238, 211, 34, 255, 40, 60, 70, 255];
        let out = binarize(&src, 150);
        assert_eq!(&out[0..3], &[0, 0, 0]); // cyan sáng -> đen (chữ)
        assert_eq!(&out[4..7], &[255, 255, 255]); // nền -> trắng
        assert_eq!(out.len(), src.len());
    }

    #[test]
    fn diem_sach_phat_hien_chu_bi_bam_nat() {
        // bản đọc đúng phải hơn hẳn bản đầy ký tự lạ
        assert!(
            cleanliness("all we saw were abandoned builds", true)
                > cleanliness("the od or•clge •s fa •rg", true)
        );
        // dấu câu thường KHÔNG bị phạt
        assert_eq!(cleanliness("ab.", true), cleanliness("ab", true));
    }

    #[test]
    fn loai_dong_rac_giu_dong_that() {
        // đúng dòng rác người dùng gặp khi xem video Minecraft
        let junk = "' oanqCảaTôi Phả Server Minecraft Bi Bo H";
        let real = "all we saw were abandoned builds";
        assert!(clean_ratio(junk, true) < 0.6, "{}", clean_ratio(junk, true));
        assert!(clean_ratio(real, true) >= 0.6);
        assert_eq!(drop_junk_lines(&format!("{real}\n{junk}"), true, 0.6), real);
        // ngưỡng 0 = tắt lọc
        assert_eq!(drop_junk_lines(junk, true, 0.0), junk);
    }

    #[test]
    fn khong_loai_nham_cau_that() {
        // dấu ba chấm / dấu câu dày không được biến câu ngắn thành rác
        for s in ["Hmm...", "Wait — what?!", "OK!", "\"Run,\" he said."] {
            assert!(clean_ratio(s, true) >= 0.6, "{s} -> {}", clean_ratio(s, true));
        }
        // chế độ Trung: chữ ngoài ASCII là BÌNH THƯỜNG, không được phạt
        let zh = "我们看到的只是废弃的建筑";
        assert!(clean_ratio(zh, false) >= 0.6);
        assert!(clean_ratio(zh, true) < 0.6); // cùng chuỗi mà bật chế độ Latin thì là rác
    }

    #[test]
    fn chu_co_dau_lac_vao_khung_tieng_anh_bi_phat() {
        // một chữ có dấu lẻ trong câu dài vẫn qua (tên riêng: "Pokémon", "café")
        assert!(clean_ratio("Pokémon trainers gather at the gym", true) >= 0.6);
        // nhưng dày đặc thì không
        assert!(clean_ratio("Cả Tôi Phả Bi Bo", true) < 0.6);
    }
}
