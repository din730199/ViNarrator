//! Nhận dạng tiếng nói bằng **Speechmatics** (dịch vụ đám mây, cần API key).
//!
//! Dùng khi model chạy máy (xem [`crate::asr`]) nghe không ra — nhạc nền to, giọng lạ, nhiều
//! người nói chồng nhau. Đổi lại: cần mạng, cần key, và **lời thoại trong game được gửi lên
//! máy chủ của họ**.
//!
//! Hai cái được thêm so với bản chạy máy: nghe được nhiều thứ tiếng (không chỉ Anh), và trả
//! về **chữ đã có sẵn dấu câu** nên [`crate::asr::Gom`] cắt câu được ngay, khỏi cần model dấu
//! câu 7,6 MB.
//!
//! **Bản ĐỒNG BỘ, không tokio.** Bản gốc (RiTaiAI) chạy adapter trên tokio vì app đó đã có sẵn
//! runtime; ở đây khâu nghe vốn đã nằm trong thread riêng của nó, kéo cả tokio vào chỉ để phục
//! vụ một socket là thừa.

use std::net::TcpStream;
use std::time::Duration;

use tungstenite::stream::MaybeTlsStream;
use tungstenite::{client::IntoClientRequest, Message, WebSocket};

const ENDPOINT: &str = "wss://eu.rt.speechmatics.com/v2";

/// Speechmatics nhận PCM 16-bit little-endian, và **16 kHz** là mức nó xử lý tốt nhất cho
/// tiếng nói. Loopback thường chạy 48 kHz nên phải hạ mẫu — xem [`ha_mau`].
pub const SR_GUI: u32 = 16_000;

/// Chờ máy chủ trả lời lúc bắt nhịp. Key sai thì nó đóng ngay nên mốc này chỉ để khỏi treo
/// vĩnh viễn khi mạng lặng thinh.
const CHO_MO: Duration = Duration::from_secs(15);

fn start_recognition(ngon_ngu: &str) -> String {
    serde_json::json!({
        "message": "StartRecognition",
        "audio_format": { "type": "raw", "encoding": "pcm_s16le", "sample_rate": SR_GUI },
        // enable_partials=false: app chỉ đọc câu đã chốt, chữ tạm chỉ tổ tốn băng thông.
        // max_delay 1.0s là mức thấp nhất họ cho, hợp với việc thuyết minh theo thời gian thực.
        "transcription_config": { "language": ngon_ngu, "enable_partials": false, "max_delay": 1.0 },
    })
    .to_string()
}

pub enum Tin {
    Batdau,
    Chu(String),
    Loi(String),
    Bo,
}

/// Chốt "câu đã xong" theo LOẠI message chứ không theo cờ nào trong đó: `AddTranscript` là bản
/// chốt, `AddPartialTranscript` là chữ tạm (đã tắt nhưng vẫn bỏ qua cho chắc).
pub fn doc_tin(raw: &str) -> Tin {
    let Ok(j) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Tin::Bo;
    };
    match j["message"].as_str() {
        Some("RecognitionStarted") => Tin::Batdau,
        Some("AddTranscript") => match j["metadata"]["transcript"].as_str() {
            Some(t) if !t.trim().is_empty() => Tin::Chu(t.to_string()),
            _ => Tin::Bo,
        },
        Some("Error") => Tin::Loi(j["reason"].as_str().unwrap_or("không rõ").to_string()),
        _ => Tin::Bo,
    }
}

/// Hạ mẫu về [`SR_GUI`] và đổi sang PCM 16-bit little-endian.
///
/// Lấy TRUNG BÌNH cả khoảng thay vì nhặt một mẫu đại diện: nhặt mẫu là lấy trộm tần số cao
/// xuống dải nghe được (răng cưa), nghe như tiếng rít lẫn vào giọng và làm bộ nhận dạng sai
/// thêm. Trung bình là bộ lọc thông thấp thô sơ nhưng đủ cho việc này.
pub fn ha_mau(mau: &[f32], sr_vao: u32) -> Vec<u8> {
    if mau.is_empty() {
        return Vec::new();
    }
    let ty = sr_vao as f64 / f64::from(SR_GUI);
    let n = (mau.len() as f64 / ty) as usize;
    let mut ra = Vec::with_capacity(n * 2);
    for i in 0..n {
        let dau = (i as f64 * ty) as usize;
        let cuoi = (((i + 1) as f64 * ty) as usize).min(mau.len()).max(dau + 1);
        let tb: f32 = mau[dau..cuoi].iter().sum::<f32>() / (cuoi - dau) as f32;
        ra.extend_from_slice(&((tb.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    ra
}

pub struct Phien {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
}

impl Phien {
    /// Mở socket và CHỜ máy chủ xác nhận đã bắt đầu. Chờ ở đây chứ không trả về ngay: key sai
    /// hay hết hạn mức thì phải báo lỗi lúc người dùng vừa bật, chứ không phải để họ ngồi nhìn
    /// màn hình im lặng rồi tự đoán.
    pub fn mo(key: &str, ngon_ngu: &str) -> Result<Self, String> {
        if key.trim().is_empty() {
            return Err("chưa nhập API key Speechmatics".into());
        }
        let mut req = ENDPOINT
            .into_client_request()
            .map_err(|e| format!("địa chỉ Speechmatics hỏng: {e}"))?;
        req.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", key.trim())
                .parse()
                .map_err(|_| "API key có ký tự không hợp lệ".to_string())?,
        );
        let (mut ws, _) = tungstenite::connect(req)
            .map_err(|e| format!("không kết nối được Speechmatics: {e}"))?;
        ws.send(Message::Text(start_recognition(ngon_ngu)))
            .map_err(|e| format!("gửi StartRecognition lỗi: {e}"))?;

        cam_gio(&mut ws, Some(CHO_MO))?;
        loop {
            match ws.read() {
                Ok(Message::Text(t)) => match doc_tin(&t) {
                    Tin::Batdau => break,
                    Tin::Loi(r) => return Err(format!("Speechmatics từ chối: {r}")),
                    _ => continue,
                },
                Ok(Message::Close(_)) => {
                    return Err("Speechmatics đóng kết nối — nhiều khả năng API key sai".into())
                }
                Ok(_) => continue,
                Err(e) => return Err(format!("chờ Speechmatics trả lời: {e}")),
            }
        }
        // Từ đây đọc KHÔNG chặn: vòng lặp còn phải bơm tiếng và ngó cờ dừng, không được ngồi
        // đợi máy chủ nói.
        cam_gio(&mut ws, None)?;
        Ok(Self { ws })
    }

    pub fn gui(&mut self, pcm: Vec<u8>) -> Result<(), String> {
        self.ws
            .send(Message::Binary(pcm))
            .map_err(|e| format!("gửi tiếng lên Speechmatics lỗi: {e}"))
    }

    /// Rút hết chữ đang chờ. `Err` = kết nối chết hẳn.
    ///
    /// Trả về Vec chứ không nhận callback: bên gọi cần đẩy chữ vào bộ gom (một thao tác ghi),
    /// mà callback ghi được thì phải là `FnMut` — kéo theo mượn khả biến lồng nhau cho một
    /// danh sách nhiều nhất vài dòng.
    pub fn nhan(&mut self) -> Result<Vec<String>, String> {
        let mut ra = Vec::new();
        loop {
            match self.ws.read() {
                Ok(Message::Text(t)) => match doc_tin(&t) {
                    Tin::Chu(c) => ra.push(c),
                    Tin::Loi(r) => return Err(format!("Speechmatics báo lỗi: {r}")),
                    _ => {}
                },
                Ok(Message::Close(_)) => return Err("Speechmatics đã đóng kết nối".into()),
                Ok(_) => {}
                // Hết chữ để đọc -> quay ra bơm tiếng tiếp. Đây là đường ra BÌNH THƯỜNG.
                Err(tungstenite::Error::Io(e)) if khong_co_gi(&e) => return Ok(ra),
                Err(e) => return Err(format!("đọc từ Speechmatics: {e}")),
            }
        }
    }
}

fn khong_co_gi(e: &std::io::Error) -> bool {
    matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
}

/// Đặt (hoặc bỏ) hạn đọc trên socket bên dưới lớp TLS.
fn cam_gio(ws: &mut WebSocket<MaybeTlsStream<TcpStream>>, han: Option<Duration>) -> Result<(), String> {
    let tcp = match ws.get_mut() {
        MaybeTlsStream::Plain(s) => s,
        MaybeTlsStream::Rustls(s) => s.get_mut(),
        _ => return Ok(()),
    };
    match han {
        Some(_) => tcp.set_nonblocking(false).map_err(|e| e.to_string())?,
        None => tcp.set_nonblocking(true).map_err(|e| e.to_string())?,
    }
    tcp.set_read_timeout(han).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_dung_loai_tin() {
        assert!(matches!(doc_tin(r#"{"message":"RecognitionStarted"}"#), Tin::Batdau));
        let t = r#"{"message":"AddTranscript","metadata":{"transcript":"Look out."}}"#;
        assert!(matches!(doc_tin(t), Tin::Chu(c) if c == "Look out."));
        // Chữ TẠM không được coi là câu chốt — nhận nhầm là đọc đi đọc lại nửa câu.
        let p = r#"{"message":"AddPartialTranscript","metadata":{"transcript":"Look"}}"#;
        assert!(matches!(doc_tin(p), Tin::Bo));
        assert!(matches!(doc_tin(r#"{"message":"Error","reason":"not authorised"}"#), Tin::Loi(_)));
        assert!(matches!(doc_tin("{ hỏng"), Tin::Bo));
        // transcript rỗng/toàn khoảng trắng -> bỏ, đừng đẩy đi dịch.
        let r = r#"{"message":"AddTranscript","metadata":{"transcript":"  "}}"#;
        assert!(matches!(doc_tin(r), Tin::Bo));
    }

    #[test]
    fn ha_mau_dung_do_dai_va_bien_do() {
        // 48k -> 16k: đúng 3 lấy 1, và mỗi mẫu ra 2 byte.
        let vao = vec![0.5f32; 4800];
        let ra = ha_mau(&vao, 48_000);
        assert_eq!(ra.len(), 1600 * 2, "sai số mẫu sau khi hạ tần");
        let m = i16::from_le_bytes([ra[0], ra[1]]);
        assert!((m - 16383).abs() < 2, "biên độ lệch: {m}");
        assert!(ha_mau(&[], 48_000).is_empty());
    }

    /// Biên độ vượt khung phải bị KẸP chứ không được cuộn vòng — cuộn vòng biến đỉnh tiếng
    /// thành tiếng rè cực to.
    #[test]
    fn kep_bien_do_khong_cuon_vong() {
        let ra = ha_mau(&[9.0; 3], 48_000);
        assert_eq!(i16::from_le_bytes([ra[0], ra[1]]), 32767);
        let ra = ha_mau(&[-9.0; 3], 48_000);
        assert_eq!(i16::from_le_bytes([ra[0], ra[1]]), -32767);
    }

    #[test]
    fn khong_co_key_thi_bao_ngay() {
        let Err(e) = Phien::mo("  ", "en") else { panic!("key rỗng mà vẫn mở kết nối") };
        assert!(e.contains("chưa nhập API key"), "báo lỗi khác: {e}");
    }
}
