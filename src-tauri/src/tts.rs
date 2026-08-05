//! TtsEngine — đọc tuần tự, ba giọng để chọn:
//! - **windows** — Windows.Media.SpeechSynthesis offline (Microsoft An). Có sẵn, 0 MB.
//! - **piper** — giọng neural VITS, xem [`crate::piper`]. Mặc định khi muốn giọng máy nhẹ.
//! - **vieneu** — giọng neural 48 kHz đọc lẫn Anh–Việt, xem [`crate::vieneu`]. Nặng nhất.
//!
//! Hai engine đầu trả WAV đầy đủ rồi phát một lần. VieNeu **phát theo mẩu**: chờ cả câu thì
//! mất 3,5s, phát từng mẩu thì nghe thấy tiếng sau ~0,2s. Vì vậy nó tự cầm khâu phát chứ
//! không trả WAV về như hai cái kia.
//!
//! Worker thread + PlaySound đồng bộ => đọc tuần tự, không chồng tiếng.
//! Chống trùng + cap qua SpeechQueue.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use serde::Serialize;
use windows::core::{HSTRING, PCWSTR};
use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceInformation};
use windows::Storage::Streams::DataReader;
use windows::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_PURGE, SND_SYNC};

use crate::ensure_com;
use crate::piper::{self, Piper};
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
}

impl TtsEngine {
    pub fn new(engine: String, voice_name: String, speed: f64, volume: u8) -> Self {
        let engine = Self {
            queue: Arc::new(SpeechQueue::new(3)),
            params: Arc::new(Mutex::new(Params { engine, voice_name, speed, volume })),
            wake: Arc::new((Mutex::new(false), Condvar::new())),
            quit: Arc::new(AtomicBool::new(false)),
            interrupt: Arc::new(AtomicBool::new(false)),
        };
        let (queue, params, wake, quit, interrupt) = (
            engine.queue.clone(),
            engine.params.clone(),
            engine.wake.clone(),
            engine.quit.clone(),
            engine.interrupt.clone(),
        );
        std::thread::spawn(move || {
            ensure_com();
            // Giữ nguyên synthesizer giữa các câu: dựng mới + `AllVoices()` mỗi câu là phí,
            // giọng chỉ đổi khi người dùng bấm dropdown. Piper cũng vậy: tiến trình con giữ
            // sống để khỏi nạp lại model 60 MB (0,48s) ở mỗi câu.
            let mut speaker: Option<Speaker> = None;
            let mut piper: Option<Piper> = None;
            let mut vieneu: Option<VieNeu> = None;
            while !quit.load(Ordering::Relaxed) {
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
                if let Err(e) = doc(&mut speaker, &mut piper, &mut vieneu, &p, &text, &interrupt) {
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
/// VieNeu phát theo mẩu nên không thể trả WAV về như hai engine kia — nó nằm riêng ở đây,
/// còn `synth` bên dưới chỉ lo hai engine "một WAV một câu".
///
/// Trục trặc ở tầng nào cũng **lùi xuống tầng dưới cho chính câu đó**: nghe thấy giọng lạ thì
/// người dùng còn biết đường đi kiểm tra, chứ im lặng thì tưởng app hỏng hẳn.
fn doc(
    speaker: &mut Option<Speaker>,
    piper_cache: &mut Option<Piper>,
    vieneu_cache: &mut Option<VieNeu>,
    p: &Params,
    text: &str,
    stop: &AtomicBool,
) -> Result<(), String> {
    if p.engine == "vieneu" && vieneu::ready() {
        match speak_vieneu(vieneu_cache, p, text, stop) {
            Ok(()) => return Ok(()),
            Err(e) => {
                eprintln!("[TTS] VieNeu lỗi: {e} — câu này đọc bằng giọng khác");
                *vieneu_cache = None; // dựng lại tiến trình con ở câu sau
            }
        }
    } else if vieneu_cache.is_some() {
        // Đổi sang engine khác -> thả worker Python, nó ăn tới ~1,3 GB RAM.
        *vieneu_cache = None;
    }
    let wav = synth(speaker, piper_cache, p, text)?;
    play_blocking(&wav);
    Ok(())
}

fn speak_vieneu(
    cache: &mut Option<VieNeu>,
    p: &Params,
    text: &str,
    stop: &AtomicBool,
) -> Result<(), String> {
    // Giọng là tham số dòng lệnh lúc spawn -> đổi giọng phải dựng lại tiến trình.
    if cache.as_ref().is_some_and(|v| v.voice() != p.voice_name) {
        *cache = None;
    }
    if cache.is_none() {
        *cache = Some(VieNeu::start(&p.voice_name)?);
    }
    // VieNeu tự cầm khâu phát (hàng đợi waveOut liền mạch) nên không trả WAV về đây.
    // Số giây trả về chỉ dùng cho test đo khe hở.
    cache.as_mut().expect("vừa gán ở trên").speak(text, p.volume, stop).map(|_| ())
}

/// Hai engine trả "một WAV cho cả câu". Piper trục trặc thì lùi về giọng Windows.
fn synth(
    speaker: &mut Option<Speaker>,
    piper_cache: &mut Option<Piper>,
    p: &Params,
    text: &str,
) -> Result<Vec<u8>, String> {
    if p.engine == "piper" && piper::ready() {
        match synth_piper(piper_cache, p, text) {
            Ok(wav) => return Ok(wav),
            Err(e) => {
                eprintln!("[TTS] Piper lỗi: {e} — câu này đọc bằng giọng Windows");
                *piper_cache = None; // dựng lại tiến trình con ở câu sau
            }
        }
    } else if piper_cache.is_some() {
        *piper_cache = None; // đổi sang giọng Windows -> thả tiến trình con, khỏi ăn RAM
    }
    synthesize(speaker, p, text).map_err(|e| e.to_string())
}

fn synth_piper(cache: &mut Option<Piper>, p: &Params, text: &str) -> Result<Vec<u8>, String> {
    // length_scale là tham số dòng lệnh lúc spawn -> kéo thanh tốc độ phải dựng lại tiến trình.
    if cache.as_ref().is_some_and(|x| x.speed() != p.speed) {
        *cache = None;
    }
    if cache.is_none() {
        *cache = Some(Piper::start(p.speed)?);
    }
    let mut wav = cache.as_mut().expect("vừa gán ở trên").synth(text)?;
    piper::apply_gain(&mut wav, p.volume);
    Ok(wav)
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

    fn sample_rate(wav: &[u8]) -> u32 {
        u32::from_le_bytes(wav[24..28].try_into().unwrap())
    }

    /// Phân biệt hai engine bằng TẦN SỐ LẤY MẪU chứ không tin lời hàm: Piper 22,05 kHz,
    /// giọng Windows 16 kHz. Nếu `synth` lỡ gọi nhầm engine thì con số này lộ ra ngay.
    /// Cần đã tải Piper (chạy `piper::tests::duong_that_tai_va_doc_duoc` trước).
    #[test]
    #[ignore]
    fn chon_dung_engine_theo_config() {
        crate::ensure_com();
        assert!(piper::ready(), "chưa tải Piper — chạy test duong_that_tai_va_doc_duoc trước");
        let (mut sp, mut pi) = (None, None);

        let w = synth(&mut sp, &mut pi, &params("piper"), "Cây cầu cũ đang sụp đổ").unwrap();
        assert_eq!(sample_rate(&w), 22_050, "chọn piper nhưng không ra tiếng của piper");

        let w = synth(&mut sp, &mut pi, &params("windows"), "Cây cầu cũ đang sụp đổ").unwrap();
        assert_ne!(sample_rate(&w), 22_050, "chọn giọng Windows nhưng vẫn ra tiếng piper");
        // Đổi về giọng Windows phải THẢ tiến trình con, không để nó ngồi ăn RAM.
        assert!(pi.is_none());
    }

    /// Lưới an toàn: Piper lỗi thì câu đó phải ra bằng giọng Windows, KHÔNG được im.
    /// Câu rỗng là cách ép `Piper::synth` trả Err mà không phải phá hỏng file cài.
    #[test]
    #[ignore]
    fn piper_loi_thi_lui_ve_giong_windows() {
        crate::ensure_com();
        assert!(piper::ready(), "chưa tải Piper");
        let (mut sp, mut pi) = (None, None);
        let w = synth(&mut sp, &mut pi, &params("piper"), "   ").expect("phải có tiếng, không Err");
        assert_eq!(&w[..4], b"RIFF");
        assert_ne!(sample_rate(&w), 22_050, "lẽ ra phải là giọng Windows");
    }
}
