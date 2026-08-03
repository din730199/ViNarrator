//! TtsEngine — đọc tuần tự bằng Windows.Media.SpeechSynthesis + PlaySound (thay winsound).
//! Port từ tts.py, chỉ giữ engine offline của Windows (edge/piper/mms xem README).
//!
//! Worker thread + PlaySound đồng bộ => đọc tuần tự. Chống trùng + cap qua SpeechQueue.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use serde::Serialize;
use windows::core::{HSTRING, PCWSTR};
use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceInformation};
use windows::Storage::Streams::DataReader;
use windows::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_PURGE, SND_SYNC};

use crate::ensure_com;
use crate::speech_queue::SpeechQueue;

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
    voice_name: String,
    speed: f64,
    volume: u8,
}

pub struct TtsEngine {
    queue: Arc<SpeechQueue>,
    params: Arc<Mutex<Params>>,
    wake: Arc<(Mutex<bool>, Condvar)>,
    quit: Arc<AtomicBool>,
}

impl TtsEngine {
    pub fn new(voice_name: String, speed: f64, volume: u8) -> Self {
        let engine = Self {
            queue: Arc::new(SpeechQueue::new(3)),
            params: Arc::new(Mutex::new(Params { voice_name, speed, volume })),
            wake: Arc::new((Mutex::new(false), Condvar::new())),
            quit: Arc::new(AtomicBool::new(false)),
        };
        let (queue, params, wake, quit) = (
            engine.queue.clone(),
            engine.params.clone(),
            engine.wake.clone(),
            engine.quit.clone(),
        );
        std::thread::spawn(move || {
            ensure_com();
            // Giữ nguyên synthesizer giữa các câu: dựng mới + `AllVoices()` mỗi câu là phí,
            // giọng chỉ đổi khi người dùng bấm dropdown.
            let mut speaker: Option<Speaker> = None;
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
                let p = params.lock().unwrap().clone();
                match synthesize(&mut speaker, &p, &text) {
                    Ok(wav) => play_blocking(&wav),
                    Err(e) => {
                        eprintln!("[TTS] lỗi: {e}");
                        speaker = None; // engine hỏng -> dựng lại ở câu sau
                    }
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
        unsafe {
            let _ = PlaySoundW(PCWSTR::null(), None, SND_PURGE);
        }
    }

    pub fn set_params(&self, voice_name: Option<String>, speed: Option<f64>, volume: Option<u8>) {
        let mut p = self.params.lock().unwrap();
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
