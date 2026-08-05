//! Piper — giọng neural tiếng Việt (VITS), chạy bằng `piper.exe` làm tiến trình con GIỮ SỐNG.
//!
//! Vì sao sidecar chứ không nhúng thẳng vào Rust: crate `piper-rs` kéo theo `espeak-rs-sys`,
//! mà cái đó cần **bindgen (libclang ~2,5 GB) + CMake** lúc build và nhúng tĩnh espeak-ng
//! (GPL-3.0) vào exe. Đo trên máy thật thì sidecar KHÔNG chậm hơn: spawn một lần rồi đẩy từng
//! câu qua stdin, model 60 MB chỉ nạp ở câu đầu.
//!
//! | | câu đầu (kèm nạp model) | câu sau, 32-39 ký tự | câu 144 ký tự |
//! |---|---|---|---|
//! | đo được | 0,484s | ~0,10s | 0,405s (RTF 0,057) |
//!
//! Runtime + model KHÔNG đóng gói kèm installer (~82 MB, và không phải ai cũng dùng) — tải về
//! `%LOCALAPPDATA%/ViNarrator/piper` ở lần đầu bật, giống cách OneOCR chép từ Snipping Tool.

use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

/// Bản standalone cuối cùng của rhasspy/piper có sẵn exe Windows (bản mới chỉ phát hành wheel
/// Python). Kèm luôn onnxruntime.dll + espeak-ng-data nên không cần cài gì thêm.
const RUNTIME_URL: &str =
    "https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_windows_amd64.zip";
const MODEL_URL: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/main/vi/vi_VN/vais1000/medium/vi_VN-vais1000-medium.onnx";
const MODEL_CFG_URL: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/main/vi/vi_VN/vais1000/medium/vi_VN-vais1000-medium.onnx.json";

/// Không hiện cửa sổ console đen mỗi lần dựng tiến trình con (CREATE_NO_WINDOW).
const NO_WINDOW: u32 = 0x0800_0000;
/// Câu dài nhất đo được tốn 0,405s. 15s là để phát hiện piper treo hẳn, không phải để chờ.
const SYNTH_TIMEOUT: Duration = Duration::from_secs(15);
/// Tải 82 MB: client blocking của reqwest mặc định timeout 30s -> đứt giữa chừng ở mạng chậm.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(900);

pub fn dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ViNarrator")
        .join("piper")
}

fn exe() -> PathBuf {
    dir().join("piper").join("piper.exe")
}

fn model() -> PathBuf {
    dir().join("vi.onnx")
}

/// Đủ file để chạy chưa. Dùng cho cả UI lẫn `tts.rs` (thiếu thì lùi về giọng Windows).
pub fn ready() -> bool {
    exe().exists() && model().exists() && dir().join("vi.onnx.json").exists()
}

fn fetch(url: &str, to: &PathBuf) -> Result<(), String> {
    if to.exists() {
        return Ok(());
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let mut res = client.get(url).send().map_err(|e| format!("tải {url}: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("tải {url}: HTTP {}", res.status()));
    }
    // .part rồi mới đổi tên: tải dở mà mất điện thì lần sau `ready()` không tưởng nhầm là xong.
    let part = to.with_extension("part");
    let mut f = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    res.copy_to(&mut f).map_err(|e| format!("ghi {}: {e}", part.display()))?;
    drop(f);
    std::fs::rename(&part, to).map_err(|e| e.to_string())
}

/// Tải + giải nén runtime và model nếu còn thiếu (~82 MB, chỉ lần đầu).
pub fn ensure_runtime() -> Result<(), String> {
    if ready() {
        return Ok(());
    }
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;

    if !exe().exists() {
        let zip = d.join("runtime.zip");
        fetch(RUNTIME_URL, &zip)?;
        // tar.exe (bsdtar) có sẵn từ Windows 10 1803 và giải được zip -> khỏi thêm crate `zip`.
        let st = Command::new("tar")
            .args(["-xf"])
            .arg(&zip)
            .arg("-C")
            .arg(&d)
            .creation_flags(NO_WINDOW)
            .status()
            .map_err(|e| format!("chạy tar: {e}"))?;
        if !st.success() {
            return Err(format!("giải nén lỗi (tar {st})"));
        }
        let _ = std::fs::remove_file(&zip);
        if !exe().exists() {
            return Err("giải nén xong nhưng không thấy piper.exe".into());
        }
    }
    fetch(MODEL_URL, &model())?;
    fetch(MODEL_CFG_URL, &d.join("vi.onnx.json"))?;
    Ok(())
}

pub struct Piper {
    child: Child,
    stdin: ChildStdin,
    /// Chỉ dùng làm tín hiệu "xong câu" — KHÔNG đọc đường dẫn piper in ra: đường dẫn có thể
    /// chứa tiếng Việt (`C:\Users\Nguyễn Văn A\...`) và piper in bằng bảng mã hẹp -> vỡ chữ.
    done: Receiver<()>,
    out_dir: PathBuf,
    speed: f64,
}

impl Piper {
    pub fn start(speed: f64) -> Result<Self, String> {
        if !ready() {
            return Err("chưa tải xong Piper".into());
        }
        let base = dir();
        let out_dir = base.join("out");
        // Dọn wav sót lại của lần chạy trước, nếu không `take_output` sẽ vớ nhầm file cũ.
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

        let piper_dir = base.join("piper");
        let mut child = Command::new(exe())
            .arg("-m")
            .arg(model())
            .arg("-c")
            .arg(base.join("vi.onnx.json"))
            .arg("-d")
            .arg(&out_dir)
            .arg("--espeak_data")
            .arg(piper_dir.join("espeak-ng-data"))
            // length_scale là NGHỊCH đảo tốc độ: đọc nhanh gấp đôi = âm vị ngắn đi một nửa.
            .arg("--length_scale")
            .arg(format!("{:.3}", 1.0 / speed.clamp(0.5, 2.0)))
            .arg("-q")
            .current_dir(&piper_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(NO_WINDOW)
            .spawn()
            .map_err(|e| format!("không chạy được piper.exe: {e}"))?;

        let stdin = child.stdin.take().ok_or("không lấy được stdin của piper")?;
        let stdout = child.stdout.take().ok_or("không lấy được stdout của piper")?;
        // Đọc stdout ở thread riêng để chỗ chờ dùng được `recv_timeout`: piper treo thì mất
        // đúng một câu, không kẹt vĩnh viễn cả luồng đọc.
        let (tx, done) = mpsc::channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            let mut line = String::new();
            while r.read_line(&mut line).unwrap_or(0) > 0 {
                if tx.send(()).is_err() {
                    break;
                }
                line.clear();
            }
        });

        Ok(Self { child, stdin, done, out_dir, speed })
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// Trả về WAV hoàn chỉnh (16-bit mono 22,05 kHz) để dùng chung `play_blocking` với giọng Windows.
    pub fn synth(&mut self, text: &str) -> Result<Vec<u8>, String> {
        // Một dòng stdin = một câu. Xuống dòng trong câu sẽ bị hiểu thành hai lượt tổng hợp
        // -> lệch pha giữa số câu gửi đi và số tín hiệu "xong" nhận về.
        let line = text.replace(['\r', '\n'], " ");
        let line = line.trim();
        if line.is_empty() {
            return Err("câu rỗng".into());
        }
        writeln!(self.stdin, "{line}").map_err(|e| format!("gửi câu cho piper: {e}"))?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        self.done
            .recv_timeout(SYNTH_TIMEOUT)
            .map_err(|_| "piper không trả lời (chết hoặc treo)".to_string())?;
        self.take_output()
    }

    /// Lấy file wav duy nhất trong out_dir rồi xoá — luôn đúng vì mỗi câu dọn sạch ngay sau đó.
    fn take_output(&self) -> Result<Vec<u8>, String> {
        let entry = std::fs::read_dir(&self.out_dir)
            .map_err(|e| e.to_string())?
            .flatten()
            .next()
            .ok_or("piper báo xong nhưng không sinh file wav")?;
        let wav = std::fs::read(entry.path()).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(entry.path());
        Ok(wav)
    }
}

impl Drop for Piper {
    fn drop(&mut self) {
        // Thả stdin trước: piper thấy EOF là tự thoát, kill() chỉ là chốt chặn.
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.out_dir);
    }
}

/// Vị trí byte đầu của chunk `data`. Duyệt chunk thay vì bê cứng 44: header WAV không cố định
/// (có bản chèn `LIST`), đoán sai là nhân biên độ vào giữa header -> tiếng rè.
fn data_offset(wav: &[u8]) -> Option<usize> {
    if wav.len() < 12 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let mut i = 12;
    while i + 8 <= wav.len() {
        let size = u32::from_le_bytes(wav[i + 4..i + 8].try_into().ok()?) as usize;
        if &wav[i..i + 4] == b"data" {
            return Some(i + 8);
        }
        i += 8 + size + (size & 1); // chunk lẻ byte thì có 1 byte đệm
    }
    None
}

/// Piper không có núm âm lượng (giọng Windows chỉnh qua WinRT Options) nên phải tự nhân mẫu i16.
pub fn apply_gain(wav: &mut [u8], volume: u8) {
    if volume >= 100 {
        return;
    }
    let Some(start) = data_offset(wav) else { return };
    let g = f32::from(volume) / 100.0;
    for s in wav[start..].chunks_exact_mut(2) {
        let v = f32::from(i16::from_le_bytes([s[0], s[1]])) * g;
        s.copy_from_slice(&(v as i16).to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(extra_chunk: bool) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(b"RIFF\0\0\0\0WAVE");
        v.extend(b"fmt ");
        v.extend(16u32.to_le_bytes());
        v.extend([0u8; 16]);
        if extra_chunk {
            v.extend(b"LIST");
            v.extend(3u32.to_le_bytes());
            v.extend([1, 2, 3]); // lẻ byte -> phải có 1 byte đệm
            v.push(0);
        }
        v.extend(b"data");
        v.extend(4u32.to_le_bytes());
        v.extend((-1000i16).to_le_bytes());
        v.extend(2000i16.to_le_bytes());
        v
    }

    fn samples(v: &[u8]) -> Vec<i16> {
        let s = data_offset(v).unwrap();
        v[s..].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
    }

    #[test]
    fn giam_am_luong_nhan_dung_mau() {
        let mut v = wav(false);
        apply_gain(&mut v, 50);
        assert_eq!(samples(&v), vec![-500, 1000]);
    }

    #[test]
    fn am_luong_100_khong_dung_toi() {
        let mut v = wav(false);
        let goc = v.clone();
        apply_gain(&mut v, 100);
        assert_eq!(v, goc);
    }

    /// Chốt lý do duyệt chunk: header có LIST thì offset KHÁC 44, bê cứng 44 sẽ nhân vào header.
    #[test]
    fn bo_qua_chunk_la_va_byte_dem() {
        let v = wav(true);
        assert_ne!(data_offset(&v), Some(44));
        let mut v2 = v.clone();
        apply_gain(&mut v2, 50);
        assert_eq!(samples(&v2), vec![-500, 1000]);
        // header phải nguyên vẹn
        assert_eq!(v2[..data_offset(&v).unwrap()], v[..data_offset(&v).unwrap()]);
    }

    #[test]
    fn wav_hong_thi_bo_qua_chu_khong_panic() {
        let mut v = b"khong phai wav".to_vec();
        let goc = v.clone();
        apply_gain(&mut v, 50);
        assert_eq!(v, goc);
    }

    /// Đường THẬT: tải ~82 MB + giải nén + dựng tiến trình + tổng hợp hai câu liên tiếp.
    /// `#[ignore]` vì tốn mạng và vài phút — chạy tay bằng:
    ///     cargo test --lib piper::tests::duong_that -- --ignored --nocapture
    #[test]
    #[ignore]
    fn duong_that_tai_va_doc_duoc() {
        ensure_runtime().expect("tải/giải nén runtime");
        assert!(ready(), "ensure_runtime() báo xong nhưng ready() vẫn false");

        let mut p = Piper::start(1.0).expect("dựng tiến trình piper");
        let t = std::time::Instant::now();
        let a = p.synth("Cây cầu cũ đang sụp đổ trong đêm").expect("câu 1");
        let t1 = t.elapsed();
        let t = std::time::Instant::now();
        let b = p.synth("Chưa ai từng vượt qua được cánh cổng thứ hai").expect("câu 2");
        let t2 = t.elapsed();

        for (i, w) in [&a, &b].iter().enumerate() {
            assert_eq!(&w[..4], b"RIFF", "câu {} không phải WAV", i + 1);
            let d = data_offset(w).expect("có chunk data");
            assert!(w.len() - d > 22_050, "câu {} ngắn bất thường: {} byte", i + 1, w.len() - d);
        }
        // Hai câu KHÁC nhau phải ra tiếng khác nhau — chốt việc không vớ nhầm file của câu trước.
        assert_ne!(a, b);
        println!("câu 1 (kèm nạp model): {t1:?} — câu 2: {t2:?}");
    }
}
