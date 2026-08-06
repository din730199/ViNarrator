//! Bắt tiếng loa **trừ tiếng của chính app**.
//!
//! # Vì sao phải viết tay bằng WASAPI thô
//!
//! `cpal` chỉ mở được loopback ở mức *thiết bị*: mở thiết bị PHÁT mặc định ở chế độ input là
//! nghe được mọi thứ đang ra loa — kể cả giọng đọc tiếng Việt của chính ViNarrator. Không chặn
//! thì app nghe mình đọc, dịch lại, đọc tiếp, lặp vô hạn.
//!
//! Bản trước chặn bằng cách **bịt tai suốt lúc TTS đang đọc**. Nó chống được vòng lặp nhưng trả
//! giá quá đắt, vì đọc một câu tiếng Việt mất xấp xỉ đúng thời gian câu tiếng Anh gốc:
//!
//! | | Game nói | App làm gì | Tai |
//! |---|---|---|---|
//! | 0–3s | câu **A** | nghe | mở |
//! | 3–3,5s | câu **B** bắt đầu | dịch A | mở |
//! | 3,5–6,5s | câu **B** tiếp | **đọc A** | **BỊT** |
//! | 6,8s | câu **C** | nghe | mở |
//!
//! Câu B mất trắng — và không chỉ phần tiếng lúc bịt tai: lúc mở lại còn `reset` bộ nhận dạng
//! nên đoạn đầu B đã nghe được cũng bị vứt. Thực tế nghe được A, C, E, G, cứ một câu lại hụt
//! một câu. Đó là lý do "bắt tệ hơn RiTaiAI" — RiTaiAI chỉ hiện chữ, không phát tiếng, nên nó
//! nghe được 100% thời gian.
//!
//! Windows 10 20H1 (build 19041) trở lên có đường bắt tiếng **theo tiến trình**: dựng
//! `IAudioClient` trên thiết bị ảo [`VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK`] với
//! [`PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE`] và PID của chính mình. Kết quả là dòng
//! tiếng chứa mọi thứ máy đang phát **trừ cây tiến trình ViNarrator**. Bịt tai thành thừa, và
//! vòng lặp tự-nghe-mình trở thành bất khả thi về mặt cấu trúc chứ không phải nhờ canh giờ.
//!
//! `cpal` không mở được đường này (nó chỉ biết thiết bị thật) nên phần bắt tiếng phải gọi
//! WASAPI trực tiếp. Không thêm thư viện nào — crate `windows` vốn đã có sẵn cho OCR và TTS.
//!
//! Máy cũ hơn 20H1 thì [`mo`] trả `Err`, bên gọi tự lùi về cpal + bịt tai như cũ.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use windows::core::{implement, Interface, Ref, Result as WResult, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    WAVEFORMATEX,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, BLOB, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcessId, WaitForSingleObject};
use windows::Win32::System::Variant::VT_BLOB;

/// Tần số yêu cầu Windows trả về. Chọn thẳng 16 kHz vì **cả hai bộ não đều muốn đúng mức này**:
/// sherpa khỏi phải tự lấy mẫu lại bên trong, còn Speechmatics thì đây là định dạng nó nhận.
/// Bộ trộn của Windows hạ mẫu giúp (xem [`AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM`]), khỏi tự viết.
pub const SR: u32 = 16_000;

/// `WAVE_FORMAT_IEEE_FLOAT`. Viết thẳng số thay vì bật cả feature `Win32_Media_Multimedia` chỉ
/// để lấy một hằng số 3 đã cố định từ thời WinMM.
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;

/// Đệm bắt tiếng, đơn vị 100ns. 200ms — đủ để một nhịp lỡ vì game giật không làm mất tiếng.
const DEM: i64 = 200 * 10_000;

/// Chờ Windows trả lời lúc mở. Trên máy không hỗ trợ, nó gọi lại ngay với mã lỗi.
const CHO_MO: u32 = 5_000;

/// Chờ mỗi lô tiếng. Hết giờ mà không có gì cũng không sao — quay ra ngó cờ dừng rồi chờ tiếp.
const CHO_LO: u32 = 100;

/// Dán việc đang làm vào lỗi của Windows. Mã HRESULT trần thì chẳng nói lên điều gì với người
/// dùng, mà mấy chục lời gọi ở đây đều trả cùng một kiểu lỗi.
fn loi(viec: &'static str) -> impl Fn(windows::core::Error) -> String {
    move |e| format!("{viec}: {e}")
}

/// Đang bắt tiếng. **Thả ra là dừng** (`Drop` báo dừng rồi đợi thread thoát hẳn).
pub struct Tieng {
    stop: Arc<AtomicBool>,
    tay: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Tieng {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.tay.take() {
            let _ = t.join();
        }
    }
}

/// Cái móc COM để `ActivateAudioInterfaceAsync` báo "xong rồi".
///
/// Hàm đó **luôn** chạy bất đồng bộ, không có bản đồng bộ nào, nên bắt buộc phải dựng một đối
/// tượng COM chỉ để nhận một tiếng gọi lại. Ở đây nó chỉ bật một event rồi thôi — phần lấy kết
/// quả để bên [`mo_that`] làm, cho luồng code đọc theo thứ tự trên xuống.
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Xong(HANDLE);

impl IActivateAudioInterfaceCompletionHandler_Impl for Xong_Impl {
    fn ActivateCompleted(&self, _op: Ref<IActivateAudioInterfaceAsyncOperation>) -> WResult<()> {
        unsafe { windows::Win32::System::Threading::SetEvent(self.0)? };
        Ok(())
    }
}

/// Mở loopback loại trừ chính app.
///
/// `Err` = máy không hỗ trợ (Windows cũ hơn 20H1) hoặc bị chặn — bên gọi phải tự lùi về cpal.
pub fn mo(tx: Sender<Vec<f32>>) -> Result<Tieng, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    // Kết quả mở phải quay về ĐÂY chứ không nuốt trong thread: mở hỏng thì bên gọi cần biết
    // ngay để lùi về cpal, chứ không phải ngồi im rồi tưởng là game không có tiếng.
    let (bao_tx, bao_rx) = std::sync::mpsc::channel::<Result<(), String>>();

    let tay = std::thread::spawn(move || {
        // MTA: `ActivateAudioInterfaceAsync` gọi lại trên thread của COM, không phải thread này.
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let ket = unsafe { chay(&tx, &s, &bao_tx) };
        if let Err(e) = ket {
            // Hỏng SAU khi đã báo mở thành công -> không ai đang đợi `bao_tx` nữa, gửi cũng
            // không sao (bên kia đã thả, `send` chỉ trả Err).
            let _ = bao_tx.send(Err(e));
        }
        if com.is_ok() {
            unsafe { CoUninitialize() };
        }
    });

    // Thread báo về đúng một lần: hoặc "mở xong" hoặc lỗi.
    match bao_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => Ok(Tieng { stop, tay: Some(tay) }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("mở loopback theo tiến trình không phản hồi".into()),
    }
}

/// Toàn bộ vòng đời bắt tiếng, chạy trong thread riêng.
///
/// Gộp mở + bắt vào một hàm vì `IAudioClient` và `IAudioCaptureClient` phải sống và chết trong
/// cùng một apartment COM.
unsafe fn chay(
    tx: &Sender<Vec<f32>>,
    stop: &AtomicBool,
    bao: &Sender<Result<(), String>>,
) -> Result<(), String> {
    // --- Dựng client trên thiết bị ảo, loại trừ cây tiến trình của mình ---
    let mut tham = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: unsafe { GetCurrentProcessId() },
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    // Windows đòi gói khối tham số trên vào một PROPVARIANT kiểu VT_BLOB.
    //
    // **`ManuallyDrop` ở đây là BẮT BUỘC, không phải cho gọn.** `windows` có cài `Drop` cho
    // PROPVARIANT (`extensions/Win32/System/StructuredStorage.rs:32`) và nó gọi
    // `PropVariantClear`, tức là đòi giải phóng `pBlobData` bằng bộ cấp phát của COM. Nhưng con
    // trỏ đó chỉ vào `tham` — một biến trên NGĂN XẾP, không phải COM cấp. Để nó dọn là hỏng
    // heap: tiến trình sập với STATUS_HEAP_CORRUPTION ngay khi dừng nghe, và sập ở tận chỗ
    // khác nên rất khó lần. Đã dính đúng như vậy, mất một lúc mới truy ra.
    let pv = std::mem::ManuallyDrop::new({
        let mut pv = PROPVARIANT::default();
        unsafe {
            let v = &mut *pv.Anonymous.Anonymous;
            v.vt = VT_BLOB;
            v.Anonymous.blob = BLOB {
                cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                pBlobData: std::ptr::from_mut(&mut tham).cast::<u8>(),
            };
        }
        pv
    });

    let su_kien = unsafe { CreateEventW(None, false, false, PCWSTR::null()) }
        .map_err(loi("không tạo được event chờ mở loopback"))?;
    let _khoa = TuDong(su_kien);
    let moc: IActivateAudioInterfaceCompletionHandler = Xong(su_kien).into();

    let viec = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(std::ptr::from_ref(&*pv)),
            &moc,
        )
    }
    // Máy cũ hơn Win10 20H1 rơi vào đây.
    .map_err(loi("máy không hỗ trợ bắt tiếng theo tiến trình"))?;

    if unsafe { WaitForSingleObject(su_kien, CHO_MO) } != WAIT_OBJECT_0 {
        return Err("chờ mở loopback theo tiến trình quá lâu".into());
    }
    let mut hr = windows::core::HRESULT(0);
    let mut doi = None;
    unsafe { viec.GetActivateResult(&mut hr, &mut doi) }
        .map_err(loi("không lấy được kết quả mở loopback"))?;
    hr.ok().map_err(loi("Windows từ chối mở loopback theo tiến trình"))?;
    let may: IAudioClient = doi
        .ok_or("Windows trả về loopback rỗng")?
        .cast()
        .map_err(loi("kết quả loopback không phải IAudioClient"))?;

    // --- Định dạng: 16 kHz, một kênh, số thực 32-bit ---
    //
    // Đường này KHÔNG có `GetMixFormat` (không gắn với thiết bị thật nào) nên phải tự khai, và
    // đó lại là điều may: xin thẳng thứ mình cần rồi để bộ trộn của Windows lo phần hạ mẫu +
    // trộn kênh. Bản cpal trước phải tự cộng trung bình các kênh rồi để sherpa hạ mẫu.
    let dinh_dang = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
        nChannels: 1,
        nSamplesPerSec: SR,
        nAvgBytesPerSec: SR * 4,
        nBlockAlign: 4,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    unsafe {
        may.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK
                | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                // Hai cờ này mới là thứ cho phép xin 16 kHz mono ở trên.
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            DEM,
            0,
            std::ptr::from_ref(&dinh_dang),
            None,
        )
    }
    .map_err(loi("không khởi tạo được loopback theo tiến trình"))?;

    let lo_moi = unsafe { CreateEventW(None, false, false, PCWSTR::null()) }
        .map_err(loi("không tạo được event báo có tiếng"))?;
    let _khoa2 = TuDong(lo_moi);
    unsafe { may.SetEventHandle(lo_moi) }.map_err(loi("không gắn được event báo có tiếng"))?;
    let bat: IAudioCaptureClient =
        unsafe { may.GetService() }.map_err(loi("không lấy được đường đọc tiếng"))?;
    unsafe { may.Start() }.map_err(loi("không bật được loopback"))?;

    // Từ đây trở đi mọi thứ đã chạy: báo cho bên gọi biết để nó khỏi lùi về cpal.
    let _ = bao.send(Ok(()));

    let ket = unsafe { vong(&bat, tx, stop, lo_moi) };
    let _ = unsafe { may.Stop() };

    // Thả client TRƯỚC khi `TuDong` đóng event. `SetEventHandle` để lại cho bộ máy âm thanh một
    // tham chiếu tới `lo_moi`, nó chỉ buông khi `IAudioClient` được giải phóng; mà Rust huỷ biến
    // cục bộ theo thứ tự NGƯỢC lúc khai nên mặc định `CloseHandle` chạy trước.
    //
    // Chưa quan sát được lỗi từ chỗ này (vụ sập heap đã truy ra là do PROPVARIANT, xem trên) —
    // giữ vì thứ tự đúng phải là vậy, không phải vì đã cháy.
    drop(bat);
    drop(may);
    ket
}

/// Vòng bơm tiếng. Tách riêng để dù lỗi giữa chừng vẫn `Stop()` được ở [`chay`].
unsafe fn vong(
    bat: &IAudioCaptureClient,
    tx: &Sender<Vec<f32>>,
    stop: &AtomicBool,
    lo_moi: HANDLE,
) -> Result<(), String> {
    while !stop.load(Ordering::Relaxed) {
        // Hết giờ mà chưa có tiếng là chuyện thường (không ai phát gì) -> ngó cờ dừng rồi chờ
        // tiếp, KHÔNG coi là lỗi.
        unsafe { WaitForSingleObject(lo_moi, CHO_LO) };
        loop {
            // Chỉ hỏi CÓ hay KHÔNG — số khung thật lấy từ `GetBuffer` bên dưới.
            match unsafe { bat.GetNextPacketSize() } {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => return Err(format!("đọc cỡ lô tiếng: {e}")),
            }
            let mut du_lieu = std::ptr::null_mut();
            let (mut khung, mut co) = (0u32, 0u32);
            unsafe { bat.GetBuffer(&mut du_lieu, &mut khung, &mut co, None, None) }
                .map_err(|e| format!("đọc lô tiếng: {e}"))?;
            if khung > 0 {
                // Windows báo "lô này toàn im lặng" thì đệm có thể là rác — phải tự dựng số 0,
                // đọc bừa vào sẽ thành tiếng rè và bộ nhận dạng nghe ra chữ từ hư không.
                let mau = if co & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                    vec![0.0f32; khung as usize]
                } else {
                    unsafe { std::slice::from_raw_parts(du_lieu.cast::<f32>(), khung as usize) }
                        .to_vec()
                };
                let _ = tx.send(mau);
            }
            unsafe { bat.ReleaseBuffer(khung) }.map_err(|e| format!("trả lô tiếng: {e}"))?;
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Đóng HANDLE khi ra khỏi tầm. Hai event trong [`chay`] đều cần, mà giữa chừng có mấy đường
/// thoát bằng `?` nên đóng tay là kiểu gì cũng sót một nhánh.
struct TuDong(HANDLE);

impl Drop for TuDong {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Một giây sóng sin 440 Hz, PCM 16-bit mono @ 16 kHz — to gần hết biên độ.
    fn tieng_thu() -> Vec<u8> {
        (0..SR)
            .flat_map(|i| {
                let t = i as f32 / SR as f32;
                let m = (t * 440.0 * std::f32::consts::TAU).sin() * 0.9;
                ((m * 32767.0) as i16).to_le_bytes()
            })
            .collect()
    }

    /// Bọc [`tieng_thu`] vào một file WAV trong bộ nhớ cho `PlaySoundW`.
    fn wav_thu() -> Vec<u16> {
        let pcm = tieng_thu();
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes()); // cỡ khối fmt
        w.extend_from_slice(&1u16.to_le_bytes()); // PCM
        w.extend_from_slice(&1u16.to_le_bytes()); // mono
        w.extend_from_slice(&SR.to_le_bytes());
        w.extend_from_slice(&(SR * 2).to_le_bytes()); // byte mỗi giây
        w.extend_from_slice(&2u16.to_le_bytes()); // khối căn
        w.extend_from_slice(&16u16.to_le_bytes()); // bit mỗi mẫu
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        w.extend_from_slice(&pcm);
        // PlaySoundW nhận con trỏ u16, đệm cho chẵn.
        w.resize(w.len().next_multiple_of(2), 0);
        w.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
    }

    /// Đo đỉnh biên độ loopback bắt được trong lúc `phat` đang chạy.
    fn do_dinh(phat: impl FnOnce()) -> (usize, f32) {
        let (tx, rx) = std::sync::mpsc::channel();
        let tai = match mo(tx) {
            Ok(t) => t,
            Err(e) => panic!("máy này không mở được loopback theo tiến trình: {e}"),
        };
        phat();
        // Đệm cho lô cuối kịp về.
        std::thread::sleep(Duration::from_millis(300));
        drop(tai);

        let (mut dinh, mut so_mau) = (0.0f32, 0usize);
        while let Ok(v) = rx.try_recv() {
            let v: Vec<f32> = v;
            so_mau += v.len();
            dinh = dinh.max(v.iter().fold(0.0f32, |a, m| a.max(m.abs())));
        }
        (so_mau, dinh)
    }

    /// **Cái test duy nhất thật sự chứng minh tính năng này.** Phát một tiếng rõ to từ CHÍNH
    /// tiến trình đang chạy test, rồi soi xem loopback có nghe thấy nó không. Không nghe thấy
    /// mới là đúng — đó là toàn bộ lý do tồn tại của module này.
    ///
    /// Thử cả HAI đường phát của app, vì loại trừ theo tiến trình chỉ có nghĩa nếu mọi tiếng
    /// app phát ra đều đi từ tiến trình này: [`crate::wave_out`] (VieNeu) và `PlaySoundW`
    /// (giọng Windows — engine mặc định).
    ///
    /// **Đã chạy ĐỎ để chắc nó phân biệt được**, không phải xanh vì máy đang im: đổi sang
    /// `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE` thì đỉnh nhảy lên 0,634 và test đổ.
    ///
    /// Đánh dấu `ignore` vì nó phát tiếng ra loa thật và phụ thuộc thiết bị âm thanh của máy:
    ///     cargo test --release --lib loopback -- --ignored --nocapture
    #[test]
    #[ignore]
    fn khong_nghe_thay_tieng_cua_chinh_minh() {
        let (mau_wo, dinh_wo) = do_dinh(|| {
            let im = AtomicBool::new(false);
            let mut loa = crate::wave_out::WaveOut::open(SR).expect("mở đường phát");
            loa.write(&tieng_thu(), &im).expect("phát tiếng thử");
            loa.drain(&im);
        });
        println!("wave_out : {mau_wo} mẫu, đỉnh {dinh_wo:.4}");

        let (mau_ps, dinh_ps) = do_dinh(|| {
            let wav = wav_thu();
            let _ = unsafe {
                windows::Win32::Media::Audio::PlaySoundW(
                    windows::core::PCWSTR(wav.as_ptr()),
                    None,
                    windows::Win32::Media::Audio::SND_MEMORY
                        | windows::Win32::Media::Audio::SND_SYNC,
                )
            };
        });
        println!("PlaySound: {mau_ps} mẫu, đỉnh {dinh_ps:.4}");

        for (ten, so_mau, dinh) in
            [("wave_out", mau_wo, dinh_wo), ("PlaySoundW", mau_ps, dinh_ps)]
        {
            assert!(so_mau > 0, "{ten}: không bắt được mẫu nào — test vô nghĩa");
            assert!(
                dinh < 0.05,
                "{ten}: nghe thấy tiếng của CHÍNH MÌNH (đỉnh {dinh:.3}) \
                 — loại trừ tiến trình không có tác dụng, vòng lặp tự nghe sẽ quay lại"
            );
        }
    }
}
