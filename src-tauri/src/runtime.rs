//! Tiện ích dùng chung cho các engine phải tự tải runtime về máy ([`crate::vieneu`]).
//!
//! Trước đây nằm trong `piper.rs`; tách ra khi bỏ Piper để chúng không đi theo.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::ChildStderr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Không hiện cửa sổ console đen mỗi lần dựng tiến trình con (CREATE_NO_WINDOW).
pub(crate) const NO_WINDOW: u32 = 0x0800_0000;

/// BELOW_NORMAL_PRIORITY_CLASS — **bắt buộc**, không phải tinh chỉnh cho vui.
///
/// App luôn chạy nền trong lúc người dùng đang chơi game, nên chỉ đo giây/câu là thiếu.
/// Đo lúc CPU **đang bị tranh** (5 tiến trình 1 luồng giả làm game + 1 luồng khung hình, vừa
/// kín 6 nhân vật lý i5-12400F), so với nền:
///
/// | ưu tiên tiến trình con | p50 | p99 | max (giật) |
/// |---|---|---|---|
/// | thường | +0,8% | +96,5% | **+154,6%** |
/// | **BELOW_NORMAL** | +2,1% | +14,2% | **+9,1%** |
/// | IDLE | +7,7% | +12,0% | +36,8% (quá tay, ngược lại tệ hơn) |
///
/// Ghim nhân KHÔNG thay được cách này: `SetProcessAffinityMask` đặt xong bị **chính
/// onnxruntime ghi đè** (đã kiểm: đặt `[0]`, chạy một câu, đọc lại ra `[0..11]`), còn
/// `OMP_NUM_THREADS` / `ORT_INTRA_OP_NUM_THREADS` thì không được đọc.
pub(crate) const BELOW_NORMAL: u32 = 0x0000_4000;

/// Client blocking của reqwest mặc định timeout 30s -> đứt giữa chừng ở mạng chậm.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(900);

/// Báo tiến độ ra UI. `Fn(&str)` chứ không phải AppHandle để module này không phụ thuộc Tauri.
pub(crate) type BaoTien<'a> = &'a dyn Fn(String);

/// Không báo tiến độ (chỉ dùng trong test).
#[cfg(test)]
pub(crate) fn im(_: String) {}

/// Đọc stderr của tiến trình con ra log VÀ giữ lại dòng cuối.
///
/// Trước đây worker chạy với `Stdio::null()` — traceback Python biến mất sạch, hỏng gì cũng
/// chỉ thấy "worker không trả lời". Dòng cuối được ghép vào thông báo lỗi để người dùng thấy
/// nguyên nhân thật chứ không phải triệu chứng.
pub(crate) fn log_stderr(ten: &'static str, err: ChildStderr) -> Arc<Mutex<String>> {
    let cuoi = Arc::new(Mutex::new(String::new()));
    let ghi = cuoi.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            eprintln!("[{ten}] {line}");
            *ghi.lock().unwrap() = line;
        }
    });
    cuoi
}

/// Ghép dòng stderr cuối vào thông báo lỗi, nếu có.
pub(crate) fn kem_stderr(msg: &str, cuoi: &Arc<Mutex<String>>) -> String {
    let c = cuoi.lock().unwrap();
    if c.is_empty() {
        msg.to_string()
    } else {
        format!("{msg} — {c}")
    }
}

/// Tải một file. `nhan` là tên hiển thị trên thanh trạng thái.
pub(crate) fn fetch(url: &str, to: &PathBuf, nhan: &str, bao: BaoTien) -> Result<(), String> {
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
    let tong = res.content_length();
    // .part rồi mới đổi tên: tải dở mà mất điện thì lần sau không tưởng nhầm là xong.
    let part = to.with_extension("part");
    let mut f = std::fs::File::create(&part).map_err(|e| e.to_string())?;

    // Tự lặp thay vì `copy_to` để báo được tiến độ: tải 560 MB mà thanh trạng thái đứng im
    // thì người dùng tưởng app treo.
    let mut buf = vec![0u8; 256 * 1024];
    let (mut xong, mut moc) = (0u64, 0u64);
    loop {
        let n = std::io::Read::read(&mut res, &mut buf).map_err(|e| format!("tải {nhan}: {e}"))?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut f, &buf[..n])
            .map_err(|e| format!("ghi {}: {e}", part.display()))?;
        xong += n as u64;
        // Báo mỗi 4 MB — đủ mượt mà không spam event sang webview.
        if xong - moc >= 4 << 20 {
            moc = xong;
            bao(match tong {
                Some(t) if t > 0 => format!("Đang tải {nhan}… {}/{} MB", xong >> 20, t >> 20),
                _ => format!("Đang tải {nhan}… {} MB", xong >> 20),
            });
        }
    }
    drop(f);
    std::fs::rename(&part, to).map_err(|e| e.to_string())
}

/// Đĩa còn trống bao nhiêu byte ở thư mục này (None = không hỏi được).
pub(crate) fn dia_trong(d: &Path) -> Option<u64> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut free = 0u64;
    unsafe {
        GetDiskFreeSpaceExW(&HSTRING::from(d.as_os_str()), Some(&mut free), None, None).ok()?;
    }
    Some(free)
}

/// Chặn trước khi tải: hết đĩa giữa chừng để lại đống dở, lần sau cài lại từ đầu — mà người
/// dùng không hiểu vì sao.
pub(crate) fn du_dia(d: &Path, can_mb: u64) -> Result<(), String> {
    match dia_trong(d) {
        Some(free) if free < can_mb << 20 => Err(format!(
            "ổ đĩa chỉ còn {} MB trống, cần ít nhất {can_mb} MB",
            free >> 20
        )),
        _ => Ok(()),
    }
}

/// Vị trí byte đầu của chunk `data` trong WAV. Duyệt chunk thay vì bê cứng 44: header WAV
/// không cố định (có bản chèn `LIST`), đoán sai là nhân biên độ vào giữa header -> tiếng rè.
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

/// Toàn thang mẫu i16.
const DINH: f32 = 32767.0;
/// Dưới mức này (tỉ lệ toàn thang) giữ nguyên tuyến tính — giọng nói ở mức bình thường không bị
/// đụng tới, chỉ mấy đỉnh phụ âm bật mới vào vùng nén.
const NGUONG: f32 = 0.7;

/// Nén mềm phần vượt ngưỡng, tiệm cận trần thay vì cắt phẳng. `goc` = biên độ TRƯỚC khi nhân.
///
/// **Vì sao không kẹp thẳng.** Kẹp là cắt phẳng ngọn sóng — sóng vuông sinh hài bậc cao, tai
/// nghe ra đúng tiếng rè. Ở đây phần vượt bị bóp theo `du/(du+con)`: `du = 0` cho đúng ngưỡng
/// (nối liền mạch với đoạn tuyến tính, độ dốc cũng bằng 1 nên không có chỗ gãy), `du` càng lớn
/// càng tiệm cận 1.0 mà **không bao giờ chạm** — nên không có mẫu nào chạm trần.
///
/// **Đo trên giọng Windows thật** (một câu 150k mẫu): đỉnh 0,96 toàn thang nhưng RMS chỉ
/// 0,097 — crest factor 19,9 dB. Đó là lý do kéo hết thanh trượt vẫn nghe nhỏ bên cạnh tiếng
/// game (game nén tới -12…-9 dBFS), và cũng là lý do **chuẩn hoá đỉnh không cứu được**: đỉnh
/// đã sát trần rồi. Chỉ **0,17%** số mẫu vượt ngưỡng 0,7, tức limiter gần như không đụng vào
/// giọng nói, chỉ gọt mấy đỉnh phụ âm bật.
///
/// `max(goc)` là vì đường cong này bóp MỌI thứ trên ngưỡng vào `[0,7; 1,0)`: mẫu vốn đã ở
/// 0,977 sẽ ra 0,964 — khuếch đại mà lại nhỏ đi. Hiếm và không nghe ra, nhưng sai về nguyên
/// tắc nên chặn sàn luôn.
fn han(v: f32, goc: f32) -> f32 {
    let a = v.abs() / DINH;
    if a <= NGUONG {
        return v;
    }
    let du = a - NGUONG;
    let con = 1.0 - NGUONG;
    let nen = (NGUONG + con * du / (du + con)).max(goc.abs() / DINH);
    v.signum() * nen * DINH
}

/// Nhân biên độ mẫu i16 — **cả đường giảm lẫn đường tăng**.
///
/// Trước đây hàm này thoát sớm khi `volume >= 100` và giọng Windows thì đi qua
/// `SetAudioVolume`, mà trần của WinRT đúng bằng 1.0. Tức kéo thanh trượt lên hết cũng chỉ là
/// "không giảm gì cả", chưa bao giờ khuếch đại. Giờ mọi khâu âm lượng của **cả hai** engine
/// dồn về đây, và >100% thì nhân thật, có [`han`] đỡ phần đỉnh.
pub(crate) fn apply_gain(wav: &mut [u8], volume: u16) {
    if volume == 100 {
        return;
    }
    let Some(start) = data_offset(wav) else { return };
    let g = f32::from(volume) / 100.0;
    for s in wav[start..].chunks_exact_mut(2) {
        let goc = f32::from(i16::from_le_bytes([s[0], s[1]]));
        let v = goc * g;
        // Nhỏ hơn 1 thì không thể tràn -> khỏi nén, giữ đúng hành vi cũ của đường giảm.
        let v = if g > 1.0 { han(v, goc) } else { v };
        // `as i16` bão hoà (không cuộn vòng) nên mẫu -32768 ở sát trần vẫn an toàn.
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

    fn wav_mau(mau: &[i16]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(b"RIFF\0\0\0\0WAVE");
        v.extend(b"fmt ");
        v.extend(16u32.to_le_bytes());
        v.extend([0u8; 16]);
        v.extend(b"data");
        v.extend((mau.len() as u32 * 2).to_le_bytes());
        mau.iter().for_each(|s| v.extend(s.to_le_bytes()));
        v
    }

    /// Chốt đúng cái người dùng phàn nàn: kéo hết thanh trượt mà không to thêm. Trước khi sửa,
    /// `apply_gain` thoát sớm ở `>= 100` nên test này ra `[-1000, 2000]` y nguyên.
    #[test]
    fn tang_am_luong_thi_to_that() {
        let mut v = wav_mau(&[-1000, 2000]);
        apply_gain(&mut v, 200);
        // 2000/32767 = 6% toàn thang -> còn xa ngưỡng nén -> nhân tuyến tính đúng 2 lần
        assert_eq!(samples(&v), vec![-2000, 4000]);
    }

    /// Lý do chọn limiter thay vì kẹp cứng: không mẫu nào được chạm trần, vì chạm trần là cắt
    /// phẳng là rè. Và nhân lên thì **không mẫu nào được nhỏ đi**.
    ///
    /// Chú ý không đòi mẫu nào cũng to lên: 32000 đã là 0,977 toàn thang, muốn to hơn thì chỉ
    /// còn cách vượt trần. Giữ nguyên đã là đúng nhất có thể.
    #[test]
    fn nen_dinh_chu_khong_cat_phang() {
        let to = [20_000i16, -31_000, 32_000];
        let mut v = wav_mau(&to);
        apply_gain(&mut v, 300);
        let ra = samples(&v);
        for (i, s) in ra.iter().enumerate() {
            assert!(s.abs() < 32767, "mẫu {i} = {s} chạm trần -> cắt phẳng");
            assert!(s.abs() >= to[i].abs(), "mẫu {i} = {s} nhỏ đi so với {} ", to[i].abs());
        }
        // 20000 (0,61 toàn thang) còn dư địa -> phải to lên thấy rõ, không chỉ hoà.
        assert!(ra[0] > 28_000, "mẫu còn dư địa mà chỉ lên {} ", ra[0]);
        // Kẹp cứng sẽ cho cả ba bằng nhau (đều đụng trần); limiter phải giữ được thứ tự.
        assert!(ra[0].abs() < ra[1].abs() && ra[1].abs() < ra[2].abs(), "{ra:?} mất tương phản");
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
        let d = data_offset(&v).unwrap();
        assert_eq!(v2[..d], v[..d], "header bị sửa");
    }

    #[test]
    fn wav_hong_thi_bo_qua_chu_khong_panic() {
        let mut v = b"khong phai wav".to_vec();
        let goc = v.clone();
        apply_gain(&mut v, 50);
        assert_eq!(v, goc);
    }

    /// `GetDiskFreeSpaceExW` phải trả số THẬT — hỏng thì `du_dia` im lặng cho qua, mất tác dụng.
    #[test]
    fn hoi_duoc_dia_trong() {
        let d = std::env::temp_dir();
        let free = dia_trong(&d).expect("không hỏi được dung lượng trống");
        assert!(free > 0, "báo 0 byte trống — nghi là gọi sai API");
        assert!(du_dia(&d, 0).is_ok());
        let qua_nhieu = (free >> 20) + 1_000_000;
        assert!(du_dia(&d, qua_nhieu).is_err(), "không chặn được khi đĩa thiếu");
    }
}
