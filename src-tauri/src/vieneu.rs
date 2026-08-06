//! VieNeu-TTS v3 Turbo — giọng neural 48 kHz, đọc lẫn Anh–Việt, chạy bằng tiến trình con Python.
//!
//! Đây là engine neural DUY NHẤT của app; engine còn lại là giọng Windows có sẵn, vừa là lưới
//! an toàn khi VieNeu hỏng. Mặc định vẫn là giọng Windows — cài xong chạy được ngay, không bắt
//! ai tải 560 MB mới nghe được câu đầu.
//!
//! **Vì sao chọn nó:** `infer_stream` (không có trong model card) cho mẩu tiếng đầu sau
//! **~0,2s**, và model được huấn luyện trên ~10.000 giờ Anh–Việt nên đọc chữ tiếng Anh lẫn
//! trong câu tử tế — đúng thứ cần cho phụ đề game đầy tên riêng tiếng Anh.
//!
//! **Đo lúc CPU bị tranh** (5 tiến trình giả làm game + 1 luồng khung hình, kín 6 nhân vật lý),
//! chạy ở tiến trình riêng mức BELOW_NORMAL, so với nền: p50 **+17,7%**, p99 **+6,2%**,
//! max **+0,1%**, RAM **1,3 GB**. Nâng nền lên gấp đôi nhưng **gần như không gây giật khung
//! hình** vì tải trải đều 1,5 nhân chứ không bung ra từng nhịp.
//!
//! *(Piper `vi_VN-vais1000` từng là lựa chọn nhẹ hơn — 97 MB RAM, max +36,2% — nhưng nghe thử
//! thì đọc chữ tiếng Anh không dùng được, nên đã gỡ hẳn. Xem README mục "Vì sao bỏ Piper".)*
//!
//! **Cách nuôi Python:** tải `uv` (một file exe tĩnh) rồi để nó tự tải CPython riêng
//! (`--python-preference only-managed`) — KHÔNG dùng Python có sẵn của máy, vì app phải xác
//! định trước môi trường chứ không phụ thuộc người dùng đã cài gì.
//!
//! Cài `vieneu` bằng `--no-deps` rồi tự thêm đúng thứ cần lúc chạy: bản đầy đủ kéo theo
//! `gradio`/`librosa`/`perth`/`pandas`/`sklearn` → **713 MB**, bản rút gọn **163 MB** mà vẫn
//! tổng hợp y hệt (đã kiểm).

use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::runtime::{self, BELOW_NORMAL, NO_WINDOW};
use crate::stretch::{self, Stretch};
use crate::wave_out::{self, WaveOut};

/// Ghim phiên bản: bản `latest` có thể đổi hành vi dòng lệnh mà không báo, còn URL bản cũ thì
/// GitHub giữ mãi. Nâng tay khi cần.
const UV_URL: &str =
    "https://github.com/astral-sh/uv/releases/download/0.12.1/uv-x86_64-pc-windows-msvc.zip";
/// `vieneu` kéo theo gradio/librosa/perth/pandas/sklearn (713 MB) mà lúc chạy không đụng tới.
/// Cài `--no-deps` rồi thêm đúng từng thứ này -> 163 MB.
const DEPS: [&str; 8] = [
    "onnxruntime>=1.20.0",
    "numpy",
    "soundfile",
    "soxr",
    "tokenizers>=0.20",
    "huggingface_hub",
    "PyYAML",
    "sea-g2p>=0.7.31",
];
/// 2 luồng: đo được vừa ít nhân hơn (1,50 so với 1,93) vừa TỐN ÍT CPU HƠN (5,89 so với 8,55
/// CPU-giây/câu, bớt đồng bộ luồng) mà độ trễ gần như không đổi.
const THREADS: &str = "2";
/// Lúc CÀI: lần đầu đời còn phải tải 285 MB model nên rộng tay.
const READY_TIMEOUT_CAI: Duration = Duration::from_secs(900);
/// Lúc CHẠY: model đã có sẵn (`ensure_runtime` kéo rồi), nạp + chạy nóng đo được ~9,5s.
/// Không dùng chung mức 900s: worker kẹt sẽ treo luồng đọc 15 phút mà không ai biết.
const READY_TIMEOUT: Duration = Duration::from_secs(60);
/// Mẩu chậm nhất đo được 0,24s. 60s là để phát hiện chết hẳn, không phải để chờ.
const CHUNK_TIMEOUT: Duration = Duration::from_secs(60);

pub fn dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ViNarrator")
        .join("vieneu")
}

fn uv_exe() -> PathBuf {
    dir().join("uv.exe")
}

fn venv_py() -> PathBuf {
    dir().join("venv").join("Scripts").join("python.exe")
}

fn worker_py() -> PathBuf {
    dir().join("worker.py")
}

/// Marker ghi ở BƯỚC CUỐI của `ensure_runtime`. Không suy từ việc "có file python" — cài dở
/// (mất mạng giữa chừng, thiếu model) vẫn để lại venv trông như đủ.
fn marker() -> PathBuf {
    dir().join(".ok")
}

pub fn ready() -> bool {
    marker().exists() && venv_py().exists() && worker_py().exists()
}

/// Ghi lại `worker.py` từ bản nhúng trong exe.
///
/// **Phải gọi ở MỌI lần dựng worker, không chỉ lúc cài.** `ensure_runtime` thoát sớm khi
/// `ready()` nên nếu chỉ ghi ở đó thì cập nhật app xong vẫn chạy worker.py của bản cũ — đã
/// dính đúng lỗi này: sửa worker để nó báo danh sách giọng, cài lại app, worker cũ vẫn chạy
/// và không có giọng nào. Ghi vài KB mỗi lần dựng là rẻ hơn nhiều so với việc gỡ lỗi đó.
fn sync_worker() -> Result<(), String> {
    std::fs::write(worker_py(), include_str!("vieneu_worker.py")).map_err(|e| e.to_string())
}

/// Danh sách giọng do CHÍNH model báo, ghi ra `voices.txt` lúc worker khởi động.
/// Rỗng = chưa cài xong; UI tự ẩn dropdown giọng.
pub fn voices() -> Vec<String> {
    std::fs::read_to_string(dir().join("voices.txt"))
        .map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

/// Chạy uv với cache + thư mục Python nằm GỌN trong thư mục của app: không rải rác vào
/// `%APPDATA%` của người dùng, gỡ app là xoá sạch một chỗ.
fn uv(args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let d = dir();
    let out = Command::new(uv_exe())
        .args(args)
        .env("UV_PYTHON_INSTALL_DIR", d.join("py"))
        .env("UV_CACHE_DIR", d.join("cache"))
        .env("UV_NO_PROGRESS", "1")
        .creation_flags(NO_WINDOW | BELOW_NORMAL)
        .output()
        .map_err(|e| format!("chạy uv: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("uv lỗi ({}): {}", out.status, err.trim()));
    }
    Ok(())
}

/// Tải uv + CPython riêng + gói rồi chạy nóng một lần để kéo nốt 285 MB model.
/// Đo được: **65 giây, 561 MB trên đĩa** (uv 46 + CPython 60 + venv 163 + model 285) trên
/// mạng nhanh. Chỉ tốn ở lần đầu bật; gọi lại khi đã xong thì trả về ngay.
pub fn ensure_runtime(bao: runtime::BaoTien) -> Result<(), String> {
    if ready() {
        return Ok(());
    }
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    // 561 MB đích + cache uv trung gian + zip chưa xoá -> xin 1,5 GB cho chắc.
    runtime::du_dia(&d, 1500)?;

    if !uv_exe().exists() {
        let zip = d.join("uv.zip");
        runtime::fetch(UV_URL, &zip, "uv", bao)?;
        let st = Command::new("tar")
            .arg("-xf")
            .arg(&zip)
            .arg("-C")
            .arg(&d)
            .creation_flags(NO_WINDOW)
            .status()
            .map_err(|e| format!("chạy tar: {e}"))?;
        if !st.success() {
            return Err(format!("giải nén uv lỗi (tar {st})"));
        }
        let _ = std::fs::remove_file(&zip);
        if !uv_exe().exists() {
            return Err("giải nén xong nhưng không thấy uv.exe".into());
        }
    }

    if !venv_py().exists() {
        bao("Đang tải Python riêng cho VieNeu…".into());
        // only-managed: BẮT uv tự tải CPython. Không có cờ này nó sẽ vớ Python cài sẵn của
        // máy (đã thấy nó chọn Python 3.12 của người dùng) -> môi trường không xác định.
        uv(&[
            "venv".as_ref(),
            "--python".as_ref(),
            "3.12".as_ref(),
            "--python-preference".as_ref(),
            "only-managed".as_ref(),
            d.join("venv").as_os_str(),
        ])?;
    }

    bao("Đang cài gói VieNeu…".into());
    let py = venv_py();
    uv(&["pip".as_ref(), "install".as_ref(), "--python".as_ref(), py.as_os_str(),
         "--no-deps".as_ref(), "vieneu".as_ref()])?;
    let mut args: Vec<&std::ffi::OsStr> =
        vec!["pip".as_ref(), "install".as_ref(), "--python".as_ref(), py.as_os_str()];
    args.extend(DEPS.iter().map(|s| s.as_ref() as &std::ffi::OsStr));
    uv(&args)?;

    sync_worker()?;

    // Chạy nóng một lần: kéo nốt ~285 MB model NGAY BÂY GIỜ, lúc UI đang báo "đang cài",
    // thay vì để câu đọc đầu tiên của người dùng treo mấy phút. Dùng mức chờ CÀI (900s) vì
    // đây là lần tải model duy nhất. Cũng là lúc `voices.txt` được ghi ra.
    bao("Đang tải model VieNeu (~285 MB)…".into());
    VieNeu::start_timeout("", READY_TIMEOUT_CAI).map(|_| ())?;

    std::fs::write(marker(), "ok").map_err(|e| e.to_string())?;
    // Cache của uv chỉ dùng lúc cài, giữ lại tốn vài trăm MB vô ích.
    let _ = std::fs::remove_dir_all(d.join("cache"));
    Ok(())
}

enum Tin {
    Chunk,
    Xong,
    Giong(String),
    Loi(String),
}

/// Kết quả một câu. Tách `tre_dau` khỏi `giay` để test đo được **riêng khe hở giữa các mẩu**:
/// chờ mẩu đầu sinh ra là chi phí cố hữu, không phải lỗi phát.
#[derive(Debug, Default)]
pub struct Doc {
    /// Tổng thời lượng tiếng đã xếp vào hàng đợi.
    pub giay: f32,
    /// Từ lúc gửi câu tới lúc có mẩu ĐẦU TIÊN.
    pub tre_dau: Duration,
}

pub struct VieNeu {
    child: Child,
    stdin: ChildStdin,
    tin: Receiver<Tin>,
    out_dir: PathBuf,
    /// Số thứ tự mẩu tiếp theo. Worker đặt tên file theo bộ đếm RIÊNG của nó và hai bộ đếm
    /// luôn khớp vì mỗi "C" là đúng một file.
    seq: u32,
    voice: String,
    stderr_cuoi: Arc<Mutex<String>>,
}

impl VieNeu {
    /// Dựng tiến trình con và CHỜ nó báo READY (~9,5s). Gọi từ luồng TTS nên chỗ gọi phải
    /// chấp nhận bị chặn — vì vậy [`crate::tts`] hâm nóng sẵn lúc người dùng chọn engine.
    pub fn start(voice: &str) -> Result<Self, String> {
        Self::start_timeout(voice, READY_TIMEOUT)
    }

    fn start_timeout(voice: &str, cho: Duration) -> Result<Self, String> {
        // Đồng bộ worker.py TRƯỚC mỗi lần dựng — xem `sync_worker`.
        sync_worker()?;
        let d = dir();
        let out_dir = d.join("out");
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

        let mut child = Command::new(venv_py())
            .arg(worker_py())
            .arg(&out_dir)
            .arg(THREADS)
            .arg(voice)
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUNBUFFERED", "1")
            .env("HF_HUB_DISABLE_SYMLINKS_WARNING", "1")
            .env("HF_HOME", d.join("hf"))
            .current_dir(&d)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(NO_WINDOW | BELOW_NORMAL)
            .spawn()
            .map_err(|e| format!("không chạy được worker VieNeu: {e}"))?;

        let stdin = child.stdin.take().ok_or("không lấy được stdin của worker")?;
        let stdout = child.stdout.take().ok_or("không lấy được stdout của worker")?;
        let stderr_cuoi = match child.stderr.take() {
            Some(e) => runtime::log_stderr("VieNeu", e),
            None => Arc::new(Mutex::new(String::new())),
        };
        let (tx, tin) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let l = line.trim();
                let t = match l {
                    "C" => Tin::Chunk,
                    "E" | "READY" => Tin::Xong,
                    _ if l.starts_with("V ") => Tin::Giong(l[2..].to_string()),
                    _ if l.starts_with('X') => Tin::Loi(l.trim_start_matches("X ").to_string()),
                    _ => continue,
                };
                if tx.send(t).is_err() {
                    break;
                }
            }
        });

        let me = Self {
            child,
            stdin,
            tin,
            out_dir,
            seq: 0,
            voice: voice.to_string(),
            stderr_cuoi,
        };
        // Worker in hết "V <giọng>" rồi mới tới READY. Gom giọng trên đường chờ.
        let mut giong = Vec::new();
        loop {
            match me.tin.recv_timeout(cho) {
                Ok(Tin::Giong(g)) => giong.push(g),
                // READY dùng chung nhánh Xong.
                Ok(Tin::Xong) => break,
                Ok(Tin::Loi(e)) => return Err(format!("worker lỗi lúc khởi động: {e}")),
                Ok(Tin::Chunk) => return Err("worker trả tin lạ lúc khởi động".into()),
                Err(_) => {
                    return Err(runtime::kem_stderr(
                        "worker VieNeu không khởi động được",
                        &me.stderr_cuoi,
                    ))
                }
            }
        }
        if !giong.is_empty() {
            // Ghi ra đĩa để UI đọc được mà KHÔNG cần dựng tiến trình Python (tốn ~10s).
            let _ = std::fs::write(dir().join("voices.txt"), giong.join("\n"));
        }
        Ok(me)
    }

    pub fn voice(&self) -> &str {
        &self.voice
    }

    /// Đọc một câu: mẩu nào xong là **xếp ngay vào hàng đợi âm thanh**, thiết bị tự nối lại.
    ///
    /// Đây là chỗ VieNeu thắng — chờ cả câu thì mất 3,5s, phát theo mẩu thì nghe thấy tiếng
    /// sau ~0,2s. Sinh nhanh gấp ~2,3 lần tốc độ phát nên mẩu sau luôn sẵn trước khi mẩu
    /// trước phát xong.
    ///
    /// **Bắt buộc dùng [`WaveOut`] chứ không phải `PlaySound`**: `PlaySound` mở lại thiết bị
    /// mỗi lần gọi, đo được **+71,6 ms mỗi ranh giới mẩu** (tổng +0,646s cho câu 9,44s). Và
    /// `infer_stream` cắt theo *frame* chứ không theo cụm từ, nên khe đó rơi vào giữa từ.
    ///
    /// `volume` áp thẳng lên mẫu vì VieNeu không có núm âm lượng. `speed` cũng vậy — model
    /// không có tham số nhịp đọc nên phải giãn tiếng bằng [`Stretch`] (WSOLA, giữ cao độ).
    pub fn speak(
        &mut self,
        text: &str,
        volume: u8,
        speed: f64,
        stop: &AtomicBool,
    ) -> Result<Doc, String> {
        let line = text.replace(['\r', '\n'], " ");
        let line = line.trim();
        if line.is_empty() {
            return Err("câu rỗng".into());
        }
        writeln!(self.stdin, "{line}").map_err(|e| format!("gửi câu cho worker: {e}"))?;
        self.stdin.flush().map_err(|e| e.to_string())?;

        // Mở thiết bị MỘT LẦN cho cả câu. Lỗi ở đây (không có thiết bị âm thanh) trả về
        // Err -> tts.rs lùi câu này về engine khác.
        let mut out: Option<WaveOut> = None;
        // Một bộ giãn cho CẢ CÂU, không phải mỗi mẩu một cái — nó phải bắc cầu qua ranh giới
        // mẩu, xem `stretch`. None = tốc độ 1x, đi thẳng không tốn gì.
        let mut keo: Option<Stretch> = None;
        let mut dem_ra: Vec<u8> = Vec::new();
        let mut tan_so = 0u32;
        let mut doc = Doc::default();
        let t0 = std::time::Instant::now();
        let mut ket = loop {
            match self.tin.recv_timeout(CHUNK_TIMEOUT) {
                Ok(Tin::Chunk) => {
                    let f = self.out_dir.join(format!("{:04}.wav", self.seq));
                    self.seq += 1;
                    // Bấm DỪNG giữa câu: vẫn phải rút HẾT tin của câu này khỏi kênh, nếu
                    // không câu SAU sẽ đọc nhầm mẩu thừa của câu này.
                    if stop.load(Ordering::Relaxed) {
                        let _ = std::fs::remove_file(&f);
                        continue;
                    }
                    let wav = std::fs::read(&f);
                    let _ = std::fs::remove_file(&f);
                    let Ok(mut wav) = wav else {
                        eprintln!("[VieNeu] không đọc được {}", f.display());
                        continue;
                    };
                    runtime::apply_gain(&mut wav, volume);
                    let Some((sr, pcm)) = wave_out::split_wav(&wav) else {
                        eprintln!("[VieNeu] mẩu không phải WAV hợp lệ");
                        continue;
                    };
                    if out.is_none() {
                        doc.tre_dau = t0.elapsed();
                        tan_so = sr;
                        // Tần số chỉ biết được khi có mẩu đầu -> dựng bộ giãn ở đây luôn.
                        if stretch::can_keo(speed) {
                            keo = Some(Stretch::new(sr, speed as f32));
                        }
                        match WaveOut::open(sr) {
                            Ok(w) => out = Some(w),
                            Err(e) => break Err(e),
                        }
                    }
                    let pcm = match keo.as_mut() {
                        Some(k) => {
                            dem_ra.clear();
                            k.push(pcm, &mut dem_ra);
                            &dem_ra[..]
                        }
                        None => pcm,
                    };
                    if let Err(e) = out.as_mut().expect("vừa mở ở trên").write(pcm, stop) {
                        break Err(e);
                    }
                    doc.giay += pcm.len() as f32 / (sr as f32 * 2.0); // 16-bit mono
                }
                Ok(Tin::Xong) => break Ok(doc),
                Ok(Tin::Loi(e)) => break Err(e),
                Ok(Tin::Giong(_)) => continue, // chỉ có lúc khởi động, ở đây thì bỏ qua
                Err(_) => {
                    break Err(runtime::kem_stderr(
                        "worker VieNeu không trả lời (chết hoặc treo)",
                        &self.stderr_cuoi,
                    ))
                }
            }
        };
        // Xả nốt đuôi câu còn kẹt trong bộ giãn (~50ms). Bỏ là mỗi câu cụt phụ âm cuối.
        if let (Some(k), Some(w), Ok(d)) = (keo.as_mut(), out.as_mut(), &mut ket) {
            dem_ra.clear();
            k.flush(&mut dem_ra);
            if w.write(&dem_ra, stop).is_ok() {
                d.giay += dem_ra.len() as f32 / (tan_so as f32 * 2.0);
            }
        }
        // Chờ phát hết trước khi trả về, nếu không câu sau sẽ chồng lên câu này.
        if let Some(mut w) = out {
            w.drain(stop);
        }
        ket
    }
}

impl Drop for VieNeu {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.out_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Âm lượng 0 = vẫn phát đủ độ dài nhưng im, để chạy test không ồn cả nhà.
    const IM: u8 = 0;

    /// Cài từ trạng thái TRẮNG rồi đọc thật. Tốn mạng (~560 MB) và vài phút — chạy tay:
    ///     cargo test --lib vieneu -- --ignored --nocapture
    #[test]
    #[ignore]
    fn duong_that_cai_va_doc_duoc() {
        ensure_runtime(&runtime::im).expect("cài runtime VieNeu");
        assert!(ready(), "ensure_runtime() xong nhưng ready() vẫn false");

        let mut v = VieNeu::start("").expect("dựng worker");
        // Worker phải báo danh sách giọng THẬT của model trước khi READY. Rỗng nghĩa là
        // worker.py trên đĩa đã cũ (bug từng gặp) hoặc giao thức lệch.
        let g = voices();
        assert!(g.len() >= 5, "chỉ nhận được {} giọng: {g:?}", g.len());

        let stop = AtomicBool::new(false);
        for cau in ["Coi chừng, có một con quái vật phía sau",
                    "Server Minecraft của tôi bị sập lúc ba giờ sáng"] {
            let d = v.speak(cau, IM, 1.0, &stop).expect("đọc câu");
            assert!(d.giay > 0.5, "câu {cau:?} chỉ ra {:.2}s tiếng — nghi là hỏng", d.giay);
            println!("{:.2}s tiếng, mẩu đầu sau {:?}", d.giay, d.tre_dau);
        }
    }

    /// **Chốt bug đã sửa:** phát từng mẩu bằng `PlaySound` chèn +71,6 ms mỗi ranh giới
    /// (tổng +0,646s cho câu 9,44s) vì mỗi lần gọi lại mở thiết bị âm thanh — mà `infer_stream`
    /// cắt theo frame nên khe rơi vào giữa từ, nghe giật rõ.
    ///
    /// Hàng đợi `waveOut` nối liền các mẩu, nên thời gian chạy phải xấp xỉ thời lượng tiếng.
    #[test]
    #[ignore]
    fn phat_lien_mach_khong_co_khe() {
        assert!(ready(), "chưa cài VieNeu — chạy duong_that_cai_va_doc_duoc trước");
        let mut v = VieNeu::start("").expect("dựng worker");
        let stop = AtomicBool::new(false);
        let cau = "Tất cả những gì chúng tôi thấy là các công trình bị bỏ hoang. \
                   Cây cầu cũ đang sụp đổ trong đêm, và chưa ai từng vượt qua được cánh cổng thứ hai.";

        let t = std::time::Instant::now();
        let d = v.speak(cau, IM, 1.0, &stop).expect("đọc câu");
        let wall = t.elapsed().as_secs_f32();
        // TRỪ độ trễ mẩu đầu: đó là chi phí cố hữu của streaming, không phải khe phát.
        let khe = wall - d.giay - d.tre_dau.as_secs_f32();
        println!("tiếng {:.3}s · chạy {wall:.3}s · mẩu đầu {:?} · khe {khe:+.3}s", d.giay, d.tre_dau);
        assert!(d.giay > 5.0, "câu dài mà chỉ ra {:.2}s tiếng", d.giay);
        // Bản cũ (PlaySound từng mẩu) chèn +0,646s cho 8 ranh giới = +71,6 ms mỗi cái.
        // Ngưỡng 0,15s vẫn bắt được ngay cả khi chỉ hai ranh giới bị hở trở lại.
        assert!(khe < 0.15, "phát bị chèn khe: {khe:+.3}s ngoài thời lượng tiếng");
    }

    /// Bấm DỪNG giữa câu thì KHÔNG được phát thêm mẩu nào nữa.
    #[test]
    #[ignore]
    fn dung_giua_cau_thi_im_ngay() {
        assert!(ready(), "chưa cài VieNeu — chạy duong_that_cai_va_doc_duoc trước");
        let mut v = VieNeu::start("").expect("dựng worker");
        let stop = AtomicBool::new(true); // đã bấm dừng từ trước
        let d = v
            .speak("Tất cả những gì chúng tôi thấy là các công trình bị bỏ hoang", IM, 1.0, &stop)
            .expect("vẫn phải rút hết tin, không được lỗi");
        assert_eq!(d.giay, 0.0, "đã dừng mà vẫn xếp {:.2}s tiếng vào hàng đợi", d.giay);
    }
}
