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
        // `enable_partials` BẬT, dù app không bao giờ đọc chữ tạm. Nó không dùng để hiển thị mà
        // dùng làm **tín hiệu "người ta vẫn đang nói"**: bộ gom giữ đuôi câu chưa có dấu kết và
        // tự chốt sau IM_CHOT, mà máy chủ chỉ gửi bản chốt khi có từ mới — người nói ngập ngừng
        // giữa câu là không có bản chốt nào tới, bộ gom hết giờ và ném đi nửa câu. Chữ tạm vẫn
        // chảy về trong lúc đó nên đẩy được hạn ra. Đây là cách RiTaiAI làm.
        //
        // max_delay 1.0s: họ cho 0,7–4,0 và mặc định 4,0. Lấy 1,0 như RiTaiAI — thuyết minh
        // theo thời gian thực thì chờ 4 giây mới có câu là không dùng được.
        "transcription_config": { "language": ngon_ngu, "enable_partials": true, "max_delay": 1.0 },
    })
    .to_string()
}

pub enum Tin {
    Batdau,
    /// Bản CHỐT — đem đi dịch được.
    Chu(String),
    /// Chữ TẠM. Nội dung vứt đi, chỉ giữ ý nghĩa "người ta vẫn đang nói" — xem
    /// [`start_recognition`].
    Tam,
    Loi(String),
    Bo,
}

/// Chốt "câu đã xong" theo LOẠI message chứ không theo cờ nào trong đó: `AddTranscript` là bản
/// chốt, `AddPartialTranscript` là chữ tạm.
pub fn doc_tin(raw: &str) -> Tin {
    let Ok(j) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Tin::Bo;
    };
    let chu = || j["metadata"]["transcript"].as_str().unwrap_or("").trim().to_string();
    match j["message"].as_str() {
        Some("RecognitionStarted") => Tin::Batdau,
        Some("AddTranscript") => match chu() {
            t if t.is_empty() => Tin::Bo,
            t => Tin::Chu(t),
        },
        // Chữ tạm rỗng KHÔNG tính là đang nói — máy chủ vẫn bắn đều lúc im lặng.
        Some("AddPartialTranscript") if !chu().is_empty() => Tin::Tam,
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

    /// Xếp một lô tiếng vào hàng gửi rồi đẩy đi được bao nhiêu hay bấy nhiêu.
    ///
    /// **`WouldBlock` ở đây KHÔNG phải lỗi.** Socket đang ở chế độ non-blocking (bắt buộc, vì
    /// vòng lặp còn phải ngó cờ dừng chứ không được ngồi đợi máy chủ), nên khi đệm gửi của hệ
    /// điều hành đầy thì `flush` trả về đúng cái đó. Bản đầu tôi coi nó là lỗi chí mạng nên
    /// đang nghe ngon lành thì app tắt ngóm với dòng
    /// `IO error: A non-blocking socket operation could not be completed immediately (os error
    /// 10035)`. `write` đã giữ dữ liệu trong đệm của tungstenite rồi, lần `flush` sau đẩy nốt.
    ///
    /// Bản gốc RiTaiAI không gặp chuyện này vì nó chạy trên tokio — runtime tự lo phần chờ.
    pub fn gui(&mut self, pcm: Vec<u8>) -> Result<(), String> {
        bo_qua_nghen(self.ws.write(Message::Binary(pcm)), "xếp tiếng vào hàng gửi")?;
        self.day()
    }

    /// Đẩy nốt đệm gửi. Gọi cả ở mỗi nhịp nhận, nếu không thì lô nào kẹt lại vì `WouldBlock`
    /// sẽ nằm đó tới lô sau mới được đẩy.
    pub fn day(&mut self) -> Result<(), String> {
        bo_qua_nghen(self.ws.flush(), "đẩy tiếng lên Speechmatics")
    }

    /// Rút hết chữ đang chờ. Trả về `(câu đã chốt, có ai đang nói không)`; `Err` = kết nối chết.
    ///
    /// Cờ thứ hai bật khi có chữ tạm — bên gọi dùng nó để lùi hạn chốt đuôi câu.
    ///
    /// Trả về Vec chứ không nhận callback: bên gọi cần đẩy chữ vào bộ gom (một thao tác ghi),
    /// mà callback ghi được thì phải là `FnMut` — kéo theo mượn khả biến lồng nhau cho một
    /// danh sách nhiều nhất vài dòng.
    pub fn nhan(&mut self) -> Result<(Vec<String>, bool), String> {
        let mut ra = Vec::new();
        let mut dang_noi = false;
        loop {
            match self.ws.read() {
                Ok(Message::Text(t)) => match doc_tin(&t) {
                    Tin::Chu(c) => ra.push(c),
                    Tin::Tam => dang_noi = true,
                    Tin::Loi(r) => return Err(format!("Speechmatics báo lỗi: {r}")),
                    _ => {}
                },
                Ok(Message::Close(_)) => return Err("Speechmatics đã đóng kết nối".into()),
                Ok(_) => {}
                // Hết chữ để đọc -> quay ra bơm tiếng tiếp. Đây là đường ra BÌNH THƯỜNG.
                Err(tungstenite::Error::Io(e)) if khong_co_gi(&e) => return Ok((ra, dang_noi)),
                Err(e) => return Err(format!("đọc từ Speechmatics: {e}")),
            }
        }
    }
}

fn khong_co_gi(e: &std::io::Error) -> bool {
    matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
}

/// Nuốt `WouldBlock`, giữ lại mọi lỗi thật.
fn bo_qua_nghen(r: Result<(), tungstenite::Error>, viec: &str) -> Result<(), String> {
    match r {
        Ok(()) => Ok(()),
        Err(tungstenite::Error::Io(e)) if khong_co_gi(&e) => Ok(()),
        // Cùng nghĩa "chưa gửi hết", tungstenite báo riêng khi bắt tay TLS đang dở.
        Err(tungstenite::Error::WriteBufferFull(_)) => Ok(()),
        Err(e) => Err(format!("{viec} lỗi: {e}")),
    }
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
        // Chữ TẠM không được coi là câu chốt — nhận nhầm là đọc đi đọc lại nửa câu. Nhưng cũng
        // không phải rác: nó là tín hiệu "còn đang nói" để lùi hạn chốt đuôi.
        let p = r#"{"message":"AddPartialTranscript","metadata":{"transcript":"Look"}}"#;
        assert!(matches!(doc_tin(p), Tin::Tam));
        // Chữ tạm RỖNG thì không tính — máy chủ vẫn bắn đều lúc không ai nói, tính vào là hạn
        // chốt bị đẩy mãi và đuôi câu không bao giờ được đọc.
        let pr = r#"{"message":"AddPartialTranscript","metadata":{"transcript":" "}}"#;
        assert!(matches!(doc_tin(pr), Tin::Bo));
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

    /// **Chốt đúng lỗi đã gặp khi chạy thật.** Socket ở chế độ non-blocking nên `flush` trả
    /// `WouldBlock` mỗi khi đệm gửi của hệ điều hành đầy — chuyện bình thường. Coi nó là lỗi
    /// thì đang nghe ngon lành app tắt ngóm với `os error 10035`.
    #[test]
    fn nghen_khong_phai_loi_nhung_loi_that_thi_phai_bao() {
        let nghen = std::io::Error::new(std::io::ErrorKind::WouldBlock, "10035");
        assert!(bo_qua_nghen(Err(tungstenite::Error::Io(nghen)), "gửi").is_ok());
        assert!(bo_qua_nghen(Ok(()), "gửi").is_ok());
        // Đứt kết nối thật thì KHÔNG được nuốt, nếu không app ngồi nghe cái socket đã chết.
        let dut = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "đứt");
        assert!(bo_qua_nghen(Err(tungstenite::Error::Io(dut)), "gửi").is_err());
        assert!(bo_qua_nghen(Err(tungstenite::Error::ConnectionClosed), "gửi").is_err());
    }

    #[test]
    fn khong_co_key_thi_bao_ngay() {
        let Err(e) = Phien::mo("  ", "en") else { panic!("key rỗng mà vẫn mở kết nối") };
        assert!(e.contains("chưa nhập API key"), "báo lỗi khác: {e}");
    }
}
