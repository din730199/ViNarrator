//! Nhật ký chẩn đoán ghi ra file.
//!
//! Sinh ra để trả lời đúng một câu hỏi mà nhìn màn hình không trả lời được: *câu thừa app đọc ra
//! đến từ khâu nào?* Ba nghi phạm cho cùng một triệu chứng, và chúng cần cách sửa khác hẳn nhau:
//!
//! | Thấy gì trong file | Thủ phạm |
//! |---|---|
//! | `[nghe]` ra chữ trong lúc `[tieng] đỉnh 0.0000` | bộ nhận dạng bịa chữ từ quãng im lặng |
//! | `[dich] NGUYÊN VĂN` | Google Dịch hỏng, app đọc thẳng tiếng Anh bằng giọng Việt |
//! | `[nghe]`/`[dich]` đều đúng, chỉ tới muộn | tồn đọng hàng đợi, không phải nghe sai |
//!
//! Bật sẵn chứ không cần cờ: mỗi câu thoại chỉ đẻ vài dòng, còn nhịp tiếng thì 2 giây một dòng.
//! Đầy [`TRAN`] thì xoá làm lại từ đầu — cái cần xem luôn là đoạn GẦN ĐÂY nhất.

use std::fmt::Display;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Quá cỡ này thì cắt làm lại. 4 MB đủ chứa nhiều giờ chơi.
const TRAN: u64 = 4 * 1024 * 1024;

/// `VN_LOG` đổi được chỗ ghi — test dùng để khỏi bẩn file thật của người dùng.
pub fn duong() -> PathBuf {
    if let Ok(p) = std::env::var("VN_LOG") {
        return PathBuf::from(p);
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ViNarrator")
        .join("chan_doan.log")
}

/// Mốc 0 của cột thời gian. Dùng đồng hồ ĐƠN ĐIỆU và số giây tương đối chứ không phải giờ thật:
/// thứ cần đọc ở đây là *khoảng cách* giữa các dòng, mà lại khỏi kéo thư viện định dạng ngày.
fn moc() -> Instant {
    static M: OnceLock<Instant> = OnceLock::new();
    *M.get_or_init(Instant::now)
}

fn khoa() -> &'static Mutex<()> {
    static K: OnceLock<Mutex<()>> = OnceLock::new();
    K.get_or_init(|| Mutex::new(()))
}

/// Ghi một dòng. Lỗi ghi file bị NUỐT có chủ đích — đây là công cụ chẩn đoán, hỏng nó thì thôi,
/// không được phép làm app đang chạy ngon phải dừng.
pub fn ghi(muc: &str, noi: impl Display) {
    let giay = moc().elapsed().as_secs_f32();
    let p = duong();
    let _g = khoa().lock().unwrap_or_else(|e| e.into_inner());
    if std::fs::metadata(&p).map(|m| m.len() > TRAN).unwrap_or(false) {
        let _ = std::fs::remove_file(&p);
    }
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{giay:8.2}s [{muc}] {noi}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chốt cái quan trọng nhất: nó ghi ĐƯỢC, ghi ĐÚNG chữ, và tự tạo thư mục còn thiếu.
    /// Không có test này thì lỗi im lặng (nuốt hết `Result`) sẽ thành một file rỗng lúc cần dùng.
    #[test]
    fn ghi_ra_file_that() {
        let d = std::env::temp_dir().join("vn_test_nhat_ky").join("sau").join("nua");
        let _ = std::fs::remove_dir_all(d.parent().unwrap().parent().unwrap());
        let p = d.join("chan_doan.log");
        // SAFETY: test chạy nối tiếp trong tiến trình này, không ai đọc env song song.
        unsafe { std::env::set_var("VN_LOG", &p) };

        ghi("nghe", format_args!("(Speechmatics) {:?}", "Look out."));
        ghi("dich", "NGUYÊN VĂN (Google lỗi: hết mạng)");
        unsafe { std::env::remove_var("VN_LOG") };

        let t = std::fs::read_to_string(&p).expect("phải tạo được file kể cả khi thiếu thư mục");
        let d: Vec<&str> = t.lines().collect();
        assert_eq!(d.len(), 2, "ghi thiếu dòng: {t}");
        assert!(d[0].contains(r#"[nghe] (Speechmatics) "Look out.""#), "sai nội dung: {}", d[0]);
        assert!(d[1].contains("[dich] NGUYÊN VĂN"), "sai nội dung: {}", d[1]);
        assert!(d[0].trim_start().starts_with(|c: char| c.is_ascii_digit()), "thiếu mốc giờ");
    }
}
