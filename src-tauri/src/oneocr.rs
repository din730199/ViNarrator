//! OneOCR — engine OCR của Snipping Tool (Windows 11). Chính xác hơn hẳn Windows.Media.Ocr ở
//! ảnh khó (chữ nhỏ trên nền nhiễu, tương phản thấp): đo trên 6 mẫu khó được 0.991 so với 0.625.
//!
//! Ba file `oneocr.dll`, `oneocr.onemodel`, `onnxruntime.dll` thuộc gói Snipping Tool của Microsoft
//! nên KHÔNG đóng gói kèm installer. Thay vào đó chép từ máy người dùng vào cạnh exe lúc chạy
//! (crate oneocr-rs hardcode đường dẫn model = thư mục exe).
//!
//! Không liệt kê được `C:\Program Files\WindowsApps` khi không có quyền admin (Access denied),
//! nên phải hỏi vị trí gói qua WinRT PackageManager — cách này chạy được với user thường.

use std::path::{Path, PathBuf};

use oneocr_rs::{OcrEngine, OcrOptions};
use windows::core::HSTRING;
use windows::Management::Deployment::PackageManager;

use crate::ensure_com;

const FAMILY_NAME: &str = "Microsoft.ScreenSketch_8wekyb3d8bbwe";
const FILES: [&str; 3] = ["oneocr.dll", "oneocr.onemodel", "onnxruntime.dll"];
/// Vài bản Snipping Tool để file ở thư mục con khác nhau.
const SUBDIRS: [&str; 3] = [
    "SnippingTool",
    r"SnippingTool\SnippingTool.Vision.Interop",
    "SnippingToolSandbox",
];

fn exe_dir() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "không lấy được thư mục exe".to_string())
}

/// Đã có đủ 3 file cạnh exe chưa (dùng để biết có phải chép lần đầu hay không).
pub fn runtime_ready() -> bool {
    exe_dir().is_ok_and(|d| FILES.iter().all(|f| d.join(f).exists()))
}

fn snipping_tool_dir() -> Result<PathBuf, String> {
    ensure_com();
    let pm = PackageManager::new().map_err(|e| format!("PackageManager: {e}"))?;
    let pkgs = pm
        .FindPackagesByUserSecurityIdPackageFamilyName(&HSTRING::from(""), &HSTRING::from(FAMILY_NAME))
        .map_err(|e| format!("không tìm được gói Snipping Tool: {e}"))?;
    for p in pkgs {
        let Ok(root) = p.InstalledPath() else { continue };
        let root = PathBuf::from(root.to_string());
        for sub in SUBDIRS {
            let dir = root.join(sub);
            if FILES.iter().all(|f| dir.join(f).exists()) {
                return Ok(dir);
            }
        }
    }
    Err("máy chưa cài Snipping Tool (hoặc bản quá cũ, không có OneOCR)".into())
}

/// Chép 3 file runtime vào cạnh exe nếu còn thiếu. ~110 MB, chỉ tốn ở lần chạy đầu.
pub fn ensure_runtime() -> Result<(), String> {
    let dst = exe_dir()?;
    if FILES.iter().all(|f| dst.join(f).exists()) {
        return Ok(());
    }
    let src = snipping_tool_dir()?;
    for f in FILES {
        let to = dst.join(f);
        if to.exists() {
            continue;
        }
        std::fs::copy(src.join(f), &to)
            .map_err(|e| format!("không chép được {f} vào {}: {e}", dst.display()))?;
    }
    Ok(())
}

pub struct OneOcr {
    engine: OcrEngine,
}

impl OneOcr {
    pub fn new() -> Result<Self, String> {
        ensure_runtime()?;
        let engine = OcrEngine::new_with_options(OcrOptions {
            include_word_level_details: false, // chỉ cần text + khung dòng
            ..Default::default()
        })
        .map_err(|e| format!("khởi tạo OneOCR lỗi: {e}"))?;
        Ok(Self { engine })
    }

    pub fn info(&self) -> String {
        "oneocr (Snipping Tool)".into()
    }

    /// rgba: đúng thứ tự xcap trả về (KHÔNG đảo kênh như Windows OCR).
    pub fn recognize_rgba(
        &self,
        rgba: &[u8],
        width: u32,
        height: u32,
        min_height_ratio: f32,
    ) -> Result<String, String> {
        let Some(buf) = image::RgbaImage::from_raw(width, height, rgba.to_vec()) else {
            return Ok(String::new());
        };
        let res = self
            .engine
            .run(buf.into())
            .map_err(|e| format!("OneOCR lỗi: {e}"))?;

        let lines: Vec<(f32, &str)> = res
            .lines
            .iter()
            .map(|l| {
                let b = &l.bounding_box;
                ((b.bottom_left.y - b.top_left.y).abs(), l.text.as_str())
            })
            .collect();
        let tallest = lines.iter().map(|l| l.0).fold(0.0f32, f32::max);
        Ok(lines
            .iter()
            .filter(|(h, _)| min_height_ratio <= 0.0 || tallest <= 0.0 || h / tallest >= min_height_ratio)
            .map(|(_, t)| *t)
            .collect::<Vec<_>>()
            .join("\n"))
    }
}
