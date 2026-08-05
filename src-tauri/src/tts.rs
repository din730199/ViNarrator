//! TtsEngine — đọc tuần tự, hai giọng để chọn:
//! - **windows** — Windows.Media.SpeechSynthesis offline (Microsoft An). Có sẵn, 0 MB.
//! - **vieneu** — giọng neural 48 kHz đọc lẫn Anh–Việt, xem [`crate::vieneu`].
//!
//! Giọng Windows trả WAV đầy đủ rồi phát một lần bằng `PlaySound`. VieNeu **phát theo mẩu**:
//! chờ cả câu thì mất 3,5s, phát từng mẩu thì nghe thấy tiếng sau ~0,2s. Vì vậy nó tự cầm
//! khâu phát (hàng đợi `waveOut` liền mạch) chứ không trả WAV về đây.
//!
//! Giọng Windows cũng là **lưới an toàn**: VieNeu hỏng ở bất kỳ đâu thì câu đó vẫn ra tiếng.
//!
//! Worker thread + phát đồng bộ => đọc tuần tự, không chồng tiếng.
//! Chống trùng + cap qua SpeechQueue.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use serde::Serialize;
use windows::core::{HSTRING, PCWSTR};
use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceInformation};
use windows::Storage::Streams::DataReader;
use windows::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_PURGE, SND_SYNC};

use crate::ensure_com;
use crate::speech_queue::SpeechQueue;
use crate::vieneu::{self, VieNeu};

#[derive(Clone, Serialize)]
pub struct Voice {
    /// Nhãn hiển thị trong dropdown, vd "Microsoft An (vi-VN)".
    pub label: String,
    /// DisplayName của WinRT — giá trị lưu vào config.voice_name.
    pub id: String,
}

fn vietnamese_voices() -> windows::core::Result<Vec<VoiceInformation>> {
    let mut out = Vec::new();
    for v in SpeechSynthesizer::AllVoices()? {
        if v.Language()?.to_string().to_lowercase().starts_with("vi") {
            out.push(v);
        }
    }
    Ok(out)
}

/// Chỉ giọng tiếng Việt offline của Windows (giống voices.list_voices của bản Python).
/// Rỗng = máy chưa cài giọng Việt (Settings → Speech → Manage voices → Add → Vietnamese).
pub fn list_voices() -> Vec<Voice> {
    ensure_com();
    vietnamese_voices()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| {
            let name = v.DisplayName().ok()?.to_string();
            let lang = v.Language().ok()?.to_string();
            Some(Voice { label: format!("{name} ({lang})"), id: name })
        })
        .collect()
}

/// name có thể là DisplayName hoặc label cũ -> VoiceInformation. Không khớp thì lấy giọng Việt đầu tiên.
fn resolve_voice(name: &str) -> Option<VoiceInformation> {
    let vs = vietnamese_voices().ok()?;
    vs.iter()
        .find(|v| v.DisplayName().map(|d| d.to_string() == name).unwrap_or(false))
        .or_else(|| {
            vs.iter()
                .find(|v| v.DisplayName().map(|d| name.starts_with(&d.to_string())).unwrap_or(false))
        })
        .or_else(|| vs.first())
        .cloned()
}

#[derive(Clone)]
struct Params {
    /// "windows" | "piper" | "vieneu". Engine chưa tải xong thì tự lùi về giọng Windows.
    engine: String,
    voice_name: String,
    speed: f64,
    volume: u8,
}

pub struct TtsEngine {
    queue: Arc<SpeechQueue>,
    params: Arc<Mutex<Params>>,
    wake: Arc<(Mutex<bool>, Condvar)>,
    quit: Arc<AtomicBool>,
    /// Bấm DỪNG giữa lúc VieNeu đang đọc dở một câu. `PlaySoundW(SND_PURGE)` chỉ cắt mẩu
    /// ĐANG phát; không có cờ này thì các mẩu còn lại của câu đó vẫn lần lượt phát tiếp.
    interrupt: Arc<AtomicBool>,
    /// Dựng sẵn engine lúc RẢNH thay vì đợi câu đầu tiên. VieNeu tốn ~10s nạp model; để
    /// đến lúc có câu mới dựng thì người dùng câm 10s VÀ mất luôn 2-3 câu phụ đề trôi qua
    /// (hàng đợi chỉ giữ 3).
    prewarm: Arc<AtomicBool>,
}

impl TtsEngine {
    pub fn new(engine: String, voice_name: String, speed: f64, volume: u8) -> Self {
        let engine = Self {
            queue: Arc::new(SpeechQueue::new(3)),
            params: Arc::new(Mutex::new(Params { engine, voice_name, speed, volume })),
            wake: Arc::new((Mutex::new(false), Condvar::new())),
            quit: Arc::new(AtomicBool::new(false)),
            interrupt: Arc::new(AtomicBool::new(false)),
            prewarm: Arc::new(AtomicBool::new(false)),
        };
        let (queue, params, wake, quit, interrupt, prewarm) = (
            engine.queue.clone(),
            engine.params.clone(),
            engine.wake.clone(),
            engine.quit.clone(),
            engine.interrupt.clone(),
            engine.prewarm.clone(),
        );
        std::thread::spawn(move || {
            ensure_com();
            // Giữ nguyên synthesizer giữa các câu: dựng mới + `AllVoices()` mỗi câu là phí,
            // giọng chỉ đổi khi người dùng bấm dropdown. VieNeu cũng vậy: tiến trình con giữ
            // sống để khỏi nạp lại model ở mỗi câu (~10s).
            let mut speaker: Option<Speaker> = None;
            let mut vieneu: Option<VieNeu> = None;
            while !quit.load(Ordering::Relaxed) {
                // Hâm nóng lúc RẢNH: chỉ chạy khi hàng đợi trống nên không làm trễ câu nào.
                if prewarm.swap(false, Ordering::Relaxed) && queue.is_empty() {
                    let p = params.lock().unwrap().clone();
                    if p.engine == "vieneu" && vieneu::ready() && vieneu_cache_can_dung(&vieneu, &p)
                    {
                        match VieNeu::start(&p.voice_name) {
                            Ok(v) => vieneu = Some(v),
                            // Không sao: câu đầu sẽ thử lại rồi lùi về engine khác nếu vẫn hỏng.
                            Err(e) => eprintln!("[TTS] hâm nóng VieNeu không được: {e}"),
                        }
                    }
                }
                let Some(text) = queue.try_dequeue() else {
                    let (lock, cv) = &*wake;
                    let mut signaled = lock.lock().unwrap();
                    while !*signaled && !quit.load(Ordering::Relaxed) {
                        signaled = cv.wait(signaled).unwrap();
                    }
                    *signaled = false;
                    continue;
                };
                // Xoá cờ ngắt NGAY TRƯỚC câu mới: cờ chỉ dùng để cắt câu đang đọc dở, không
                // được để nó làm câm luôn câu kế tiếp.
                interrupt.store(false, Ordering::Relaxed);
                let p = params.lock().unwrap().clone();
                if let Err(e) = doc(&mut speaker, &mut vieneu, &p, &text, &interrupt) {
                    eprintln!("[TTS] lỗi: {e}");
                    speaker = None; // engine hỏng -> dựng lại ở câu sau
                }
            }
        });
        engine
    }

    pub fn speak(&self, text: &str) {
        if self.queue.enqueue(text) {
            self.notify();
        }
    }

    /// Đọc lại chủ động: bỏ qua bộ chống trùng "câu vừa nhận".
    pub fn speak_now(&self, text: &str) {
        self.queue.reset_dedup();
        self.speak(text);
    }

    pub fn stop_playback(&self) {
        self.queue.clear();
        self.interrupt.store(true, Ordering::Relaxed);
        unsafe {
            let _ = PlaySoundW(PCWSTR::null(), None, SND_PURGE);
        }
    }

    pub fn set_params(
        &self,
        engine: Option<String>,
        voice_name: Option<String>,
        speed: Option<f64>,
        volume: Option<u8>,
    ) {
        let mut p = self.params.lock().unwrap();
        if let Some(e) = engine {
            p.engine = e;
        }
        if let Some(v) = voice_name {
            p.voice_name = v;
        }
        if let Some(s) = speed {
            p.speed = s;
        }
        if let Some(v) = volume {
            p.volume = v;
        }
    }

    /// Bảo luồng đọc dựng sẵn engine ngay khi rảnh. Gọi lúc người dùng vừa chọn engine hoặc
    /// vừa cài xong — KHÔNG chặn, việc dựng xảy ra ở luồng đọc.
    pub fn prewarm(&self) {
        self.prewarm.store(true, Ordering::Relaxed);
        self.notify();
    }

    pub fn shutdown(&self) {
        self.quit.store(true, Ordering::Relaxed);
        self.notify();
    }

    fn notify(&self) {
        let (lock, cv) = &*self.wake;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }
}

/// Synthesizer dùng lại + tên giọng ĐANG gắn vào nó (để biết khi nào cần `SetVoice`).
struct Speaker {
    synth: SpeechSynthesizer,
    voice_name: String,
}

/// Lấy synthesizer từ cache, chỉ quét `AllVoices()` khi tên giọng đổi.
/// Tốc độ/âm lượng nằm trong `Options()` nên set lại mỗi câu — chỉ là ghi field, không tốn gì.
fn speaker_for<'a>(
    cache: &'a mut Option<Speaker>,
    p: &Params,
) -> windows::core::Result<&'a SpeechSynthesizer> {
    if cache.is_none() {
        *cache = Some(Speaker {
            synth: SpeechSynthesizer::new()?,
            // rỗng != mọi DisplayName hợp lệ -> lượt đầu chắc chắn chạy SetVoice
            voice_name: String::new(),
        });
    }
    let s = cache.as_mut().expect("vừa gán ở trên");
    if s.voice_name != p.voice_name {
        if let Some(v) = resolve_voice(&p.voice_name) {
            s.synth.SetVoice(&v)?;
        }
        // Ghi nhận kể cả khi không tìm thấy giọng: máy chưa cài giọng Việt thì đừng quét lại
        // toàn bộ giọng hệ thống ở mọi câu. Đổi giọng trên UI sẽ set_params -> khác tên -> quét lại.
        s.voice_name = p.voice_name.clone();
    }
    let opts = s.synth.Options()?;
    opts.SetSpeakingRate(p.speed.clamp(0.5, 6.0))?;
    opts.SetAudioVolume(f64::from(p.volume.min(100)) / 100.0)?;
    Ok(&s.synth)
}

/// Đọc một câu bằng engine đang chọn, tự lo cả khâu phát.
///
/// VieNeu phát theo mẩu nên không trả WAV về — nó nằm riêng ở đây, còn `synthesize` bên dưới
/// lo giọng Windows kiểu "một WAV một câu".
///
/// VieNeu trục trặc thì **lùi về giọng Windows cho chính câu đó**: nghe thấy giọng lạ thì
/// người dùng còn biết đường đi kiểm tra, chứ im lặng thì tưởng app hỏng hẳn.
fn doc(
    speaker: &mut Option<Speaker>,
    vieneu_cache: &mut Option<VieNeu>,
    p: &Params,
    text: &str,
    stop: &AtomicBool,
) -> Result<(), String> {
    if p.engine == "vieneu" && vieneu::ready() {
        match speak_vieneu(vieneu_cache, p, text, stop) {
            Ok(()) => return Ok(()),
            Err(e) => {
                eprintln!("[TTS] VieNeu lỗi: {e} — câu này đọc bằng giọng Windows");
                *vieneu_cache = None; // dựng lại tiến trình con ở câu sau
            }
        }
    } else if vieneu_cache.is_some() {
        // Đổi sang giọng Windows -> thả worker Python, nó ăn tới ~1,3 GB RAM.
        *vieneu_cache = None;
    }
    let wav = synthesize(speaker, p, text).map_err(|e| e.to_string())?;
    play_blocking(&wav);
    Ok(())
}

/// Có cần dựng (lại) worker VieNeu không: chưa có, hoặc đang giữ worker của giọng khác.
fn vieneu_cache_can_dung(cache: &Option<VieNeu>, p: &Params) -> bool {
    cache.as_ref().is_none_or(|v| v.voice() != p.voice_name)
}

fn speak_vieneu(
    cache: &mut Option<VieNeu>,
    p: &Params,
    text: &str,
    stop: &AtomicBool,
) -> Result<(), String> {
    // Giọng là tham số dòng lệnh lúc spawn -> đổi giọng phải dựng lại tiến trình.
    if vieneu_cache_can_dung(cache, p) {
        *cache = None;
        *cache = Some(VieNeu::start(&p.voice_name)?);
    }
    // VieNeu tự cầm khâu phát (hàng đợi waveOut liền mạch) nên không trả WAV về đây.
    // Số giây trả về chỉ dùng cho test đo khe hở.
    cache.as_mut().expect("vừa gán ở trên").speak(text, p.volume, stop).map(|_| ())
}

fn synthesize(
    cache: &mut Option<Speaker>,
    p: &Params,
    text: &str,
) -> windows::core::Result<Vec<u8>> {
    let synth = speaker_for(cache, p)?;
    let stream = synth.SynthesizeTextToStreamAsync(&HSTRING::from(text))?.join()?;
    let size = stream.Size()? as u32;
    let reader = DataReader::CreateDataReader(&stream)?;
    reader.LoadAsync(size)?.join()?;
    let mut wav = vec![0u8; size as usize];
    reader.ReadBytes(&mut wav)?;
    Ok(wav)
}

/// SND_SYNC = chặn tới khi đọc xong -> hàng đợi phát tuần tự, không chồng tiếng.
fn play_blocking(wav: &[u8]) {
    unsafe {
        let _ = PlaySoundW(PCWSTR(wav.as_ptr() as *const u16), None, SND_MEMORY | SND_SYNC);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(engine: &str) -> Params {
        Params {
            engine: engine.into(),
            voice_name: "Microsoft An".into(),
            speed: 1.0,
            volume: 100,
        }
    }

    /// Chọn giọng Windows thì **không được** dựng worker VieNeu (nó ăn ~1,3 GB RAM).
    #[test]
    #[ignore]
    fn giong_windows_khong_dung_worker_vieneu() {
        crate::ensure_com();
        let (mut sp, mut vn) = (None, None);
        let stop = AtomicBool::new(false);
        doc(&mut sp, &mut vn, &params("windows"), "Cây cầu cũ đang sụp đổ", &stop)
            .expect("đọc bằng giọng Windows");
        assert!(vn.is_none(), "chọn giọng Windows mà vẫn dựng worker VieNeu");
        assert!(sp.is_some(), "không dựng synthesizer của Windows");
    }

    /// **Lưới an toàn:** VieNeu lỗi thì câu đó phải ra bằng giọng Windows, KHÔNG được im.
    ///
    /// Câu toàn khoảng trắng ép `VieNeu::speak` trả Err mà không phải phá file cài. Chốt bằng
    /// việc synthesizer Windows ĐƯỢC dựng: không có nhánh lùi thì `doc` trả Err và `sp` vẫn None.
    #[test]
    #[ignore]
    fn vieneu_loi_thi_lui_ve_giong_windows() {
        crate::ensure_com();
        assert!(vieneu::ready(), "chưa cài VieNeu — chạy vieneu::tests::duong_that... trước");
        let (mut sp, mut vn) = (None, None);
        let stop = AtomicBool::new(false);
        doc(&mut sp, &mut vn, &params("vieneu"), "   ", &stop).expect("phải ra tiếng, không Err");
        assert!(sp.is_some(), "VieNeu lỗi mà không lùi về giọng Windows — câu này bị câm");
    }
}
