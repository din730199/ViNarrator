//! Narrator — worker thread: chụp -> (mask loại trừ) -> OCR -> (dịch) -> đọc. Port từ narrator.py.
//! Trạng thái đẩy về UI qua event Tauri: "status" (chuỗi trạng thái) và "text" (câu vừa đọc).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use crate::capture;
use crate::config::{self, Config, Region};
use crate::dedup::LineDeduper;
use crate::ocr::Ocr;
use crate::translate::{self, Translator};
use crate::tts::TtsEngine;

/// Cờ dựng lại từng phần. Trước đây gộp làm một nên kéo thanh "nhịp quét" cũng khiến OneOCR
/// khởi tạo lại (~100-300ms) và deduper mất sạch lịch sử -> đọc lặp câu đang hiện.
#[derive(Default)]
struct Reload {
    ocr: AtomicBool,
    translate: AtomicBool,
    dedup: AtomicBool,
}

impl Reload {
    fn all() -> Self {
        Self {
            ocr: AtomicBool::new(true),
            translate: AtomicBool::new(true),
            dedup: AtomicBool::new(true),
        }
    }
}

pub struct Narrator {
    cfg: Arc<Mutex<Config>>,
    running: Arc<AtomicBool>,
    quit: Arc<AtomicBool>,
    reload: Arc<Reload>,
    last_text: Arc<Mutex<String>>,
    tts: Arc<TtsEngine>,
}

impl Narrator {
    pub fn new(app: AppHandle, cfg: Config) -> Self {
        let tts = Arc::new(TtsEngine::new(
            cfg.tts_engine.clone(),
            cfg.voice_name.clone(),
            cfg.speed,
            cfg.volume,
        ));
        let me = Self {
            cfg: Arc::new(Mutex::new(cfg)),
            running: Arc::new(AtomicBool::new(false)),
            quit: Arc::new(AtomicBool::new(false)),
            reload: Arc::new(Reload::all()),
            last_text: Arc::new(Mutex::new(String::new())),
            tts,
        };
        let w = Worker {
            app,
            cfg: me.cfg.clone(),
            running: me.running.clone(),
            quit: me.quit.clone(),
            reload: me.reload.clone(),
            last_text: me.last_text.clone(),
            tts: me.tts.clone(),
        };
        std::thread::spawn(move || w.run());
        me
    }

    pub fn config(&self) -> Config {
        self.cfg.lock().unwrap().clone()
    }

    fn edit<F: FnOnce(&mut Config)>(&self, f: F) {
        let cfg = {
            let mut g = self.cfg.lock().unwrap();
            f(&mut g);
            g.clone()
        };
        if let Err(e) = config::save(&cfg) {
            eprintln!("[config] không lưu được: {e}");
        }
    }

    pub fn running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn set_running(&self, on: bool) -> bool {
        self.running.store(on, Ordering::Relaxed);
        if !on {
            self.tts.stop_playback();
        }
        on
    }

    pub fn toggle(&self) -> bool {
        self.set_running(!self.running())
    }

    pub fn reread(&self) {
        let t = self.last_text.lock().unwrap().clone();
        if !t.is_empty() {
            self.tts.speak_now(&t);
        }
    }

    /// Đổi chế độ = đổi ngôn ngữ OCR VÀ hướng dịch, lịch sử dedup cũng thành vô nghĩa.
    pub fn set_mode(&self, mode: &str) {
        self.edit(|c| c.apply_mode(mode));
        for f in [&self.reload.ocr, &self.reload.translate, &self.reload.dedup] {
            f.store(true, Ordering::Relaxed);
        }
    }

    pub fn set_engine(&self, engine: String) {
        self.edit(|c| c.ocr_engine = engine.clone());
        self.reload.ocr.store(true, Ordering::Relaxed);
    }

    /// engine: "google" | "llm". Các field LLM chỉ có tác dụng khi engine = "llm".
    pub fn set_translate(&self, engine: String, base_url: String, model: String, api_key: String, context_lines: usize) {
        self.edit(|c| {
            c.translate_engine = engine;
            c.llm_base_url = base_url;
            c.llm_model = model;
            c.llm_api_key = api_key;
            c.llm_context_lines = context_lines;
        });
        self.reload.translate.store(true, Ordering::Relaxed);
    }

    /// Các knob tinh chỉnh trước đây chỉ sửa được bằng tay trong config.json.
    pub fn set_tuning(
        &self,
        interval_ms: u64,
        min_line_len: usize,
        min_clean_ratio: f32,
        fuzzy_threshold: f64,
    ) {
        self.edit(|c| {
            c.interval_ms = interval_ms.clamp(200, 5000);
            c.min_line_len = min_line_len.clamp(1, 20);
            c.min_clean_ratio = min_clean_ratio.clamp(0.0, 1.0);
            c.fuzzy_threshold = fuzzy_threshold.clamp(0.5, 1.0);
        });
        // interval_ms và min_clean_ratio đọc thẳng từ cfg mỗi vòng; min_line_len và
        // fuzzy_threshold nằm trong deduper nên phải dựng lại.
        self.reload.dedup.store(true, Ordering::Relaxed);
    }

    /// engine: "windows" | "piper" | "vieneu".
    pub fn set_tts_engine(&self, engine: String) {
        self.edit(|c| c.tts_engine = engine.clone());
        self.tts.set_params(Some(engine), None, None, None);
        self.tts.prewarm();
    }

    pub fn set_voice(&self, name: String) {
        self.edit(|c| c.voice_name = name.clone());
        self.tts.set_params(None, Some(name), None, None);
        // Đổi giọng VieNeu = dựng lại tiến trình con (~10s). Làm ngay lúc rảnh thay vì để
        // câu tiếp theo gánh.
        self.tts.prewarm();
    }

    /// Vừa cài xong runtime -> dựng sẵn engine luôn.
    pub fn prewarm(&self) {
        self.tts.prewarm();
    }

    pub fn set_speed(&self, speed: f64) {
        self.edit(|c| c.speed = speed);
        self.tts.set_params(None, None, Some(speed), None);
    }

    pub fn set_volume(&self, volume: u8) {
        self.edit(|c| c.volume = volume);
        self.tts.set_params(None, None, None, Some(volume));
    }

    pub fn set_region(&self, region: Region) {
        self.edit(|c| c.region = region);
    }

    pub fn set_exclude(&self, region: Option<Region>) {
        self.edit(|c| c.exclude_region = region);
    }

    pub fn shutdown(&self) {
        self.quit.store(true, Ordering::Relaxed);
        self.tts.shutdown();
    }
}

struct Worker {
    app: AppHandle,
    cfg: Arc<Mutex<Config>>,
    running: Arc<AtomicBool>,
    quit: Arc<AtomicBool>,
    reload: Arc<Reload>,
    last_text: Arc<Mutex<String>>,
    tts: Arc<TtsEngine>,
}

impl Worker {
    fn status(&self, s: impl Into<String>) {
        let _ = self.app.emit("status", s.into());
    }

    fn run(self) {
        crate::ensure_com();
        let mut ocr: Option<Ocr> = None;
        let mut deduper = {
            let c = self.cfg.lock().unwrap();
            LineDeduper::new(c.dedup_require_stable, c.min_line_len, c.fuzzy_threshold)
        };

        // Dịch + đọc chạy ở thread RIÊNG: một câu dịch bằng AI tốn 200-800ms, để chung vòng
        // chụp thì khung kế tiếp bị trễ đúng ngần ấy và phụ đề đổi trong lúc đó là mất luôn.
        let (tx, rx) = mpsc::channel::<Msg>();
        {
            let (app, tts, last_text, running) = (
                self.app.clone(),
                self.tts.clone(),
                self.last_text.clone(),
                self.running.clone(),
            );
            std::thread::spawn(move || translate_loop(app, tts, last_text, running, rx));
        }

        loop {
            if self.quit.load(Ordering::Relaxed) {
                return;
            }
            let cfg = self.cfg.lock().unwrap().clone();

            let redo_ocr = self.reload.ocr.swap(false, Ordering::Relaxed);
            let redo_translate = self.reload.translate.swap(false, Ordering::Relaxed);
            if self.reload.dedup.swap(false, Ordering::Relaxed) {
                deduper =
                    LineDeduper::new(cfg.dedup_require_stable, cfg.min_line_len, cfg.fuzzy_threshold);
            }
            if redo_translate {
                let _ = tx.send(Msg::Reload(Box::new(cfg.clone())));
            }

            if redo_ocr {
                // Lần đầu chọn OneOCR phải chép ~110MB từ gói Snipping Tool -> báo trước
                // để người dùng không tưởng app treo.
                if cfg.ocr_engine != "windows" && !crate::oneocr::runtime_ready() {
                    self.status("Đang chép engine OneOCR từ Snipping Tool (~110 MB, chỉ lần đầu)...");
                } else {
                    self.status("Đang khởi tạo OCR...");
                }
                match Ocr::new(&cfg) {
                    Ok(o) => {
                        ocr = Some(o);
                    }
                    Err(e) => {
                        self.status(format!("Lỗi khởi tạo OCR: {e}"));
                        ocr = None;
                    }
                }
                deduper.reset();
            }

            // Thanh trạng thái gộp cả hai vế nên phải cập nhật khi BẤT KỲ vế nào đổi.
            if let Some(o) = ocr.as_ref().filter(|_| redo_ocr || redo_translate) {
                self.status(format!("Sẵn sàng — {}, {}", o.info(), translate::describe(&cfg)));
            }

            let Some(engine) = ocr.as_ref().filter(|_| self.running.load(Ordering::Relaxed)) else {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            };

            let t0 = Instant::now();
            if let Err(e) = self.tick(engine, &tx, &mut deduper, &cfg) {
                self.status(format!("Lỗi: {e}"));
            }
            // Giữ nhịp ~interval (trừ thời gian OCR đã tốn) -> không cộng dồn -> nhạy hơn.
            let elapsed = t0.elapsed();
            let interval = Duration::from_millis(cfg.interval_ms);
            std::thread::sleep(interval.saturating_sub(elapsed).max(Duration::from_millis(20)));
        }
    }

    fn tick(
        &self,
        engine: &Ocr,
        tx: &Sender<Msg>,
        deduper: &mut LineDeduper,
        cfg: &Config,
    ) -> Result<(), String> {
        let (mut rgba, w, h) = capture::grab_rgba(&cfg.region)?;
        capture::mask_exclusion(&mut rgba, w, h, &cfg.region, cfg.exclude_region.as_ref());
        let text = engine.recognize_rgba(&rgba, w, h, cfg)?;
        for line in deduper.feed(&text) {
            let _ = tx.send(Msg::Line(line));
        }
        Ok(())
    }
}

enum Msg {
    Line(String),
    Reload(Box<Config>),
}

/// Câu tồn đọng tối đa. Dịch không kịp phụ đề thì thà bỏ câu cũ còn hơn đọc trễ cả phút —
/// cùng lý do hàng đợi TTS cap 3.
const MAX_PENDING: usize = 3;

/// Một mục log: giữ cả bản gốc để phân biệt lỗi OCR với lỗi dịch.
#[derive(Clone, serde::Serialize)]
struct Spoken {
    src: String,
    out: String,
}

fn translate_loop(
    app: AppHandle,
    tts: Arc<TtsEngine>,
    last_text: Arc<Mutex<String>>,
    running: Arc<AtomicBool>,
    rx: Receiver<Msg>,
) {
    let mut translator = Translator::new(&Config::default());
    // recv() lỗi = thread chụp đã chết -> thoát luôn.
    while let Ok(first) = rx.recv() {
        // Gom hết thứ đã tới trong lúc đang bận dịch câu trước, rồi mới quyết bỏ cái nào.
        let mut pending = Vec::new();
        let mut msg = Some(first);
        while let Some(m) = msg {
            match m {
                Msg::Reload(c) => translator = Translator::new(&c),
                Msg::Line(l) => pending.push(l),
            }
            msg = rx.try_recv().ok();
        }
        // Bấm DỪNG là phải im NGAY. `stop_playback()` chỉ xoá hàng đợi TTS; nếu ở đây cứ dịch
        // tiếp rồi `speak()` thì nạp lại vào đúng cái hàng đợi vừa xoá -> vẫn nghe thêm mấy câu
        // sau khi đã bấm dừng (dịch bằng AI thì kéo dài tới ~2,5s).
        if !running.load(Ordering::Relaxed) {
            if !pending.is_empty() {
                eprintln!("[translate] dừng đọc — bỏ {} câu đang chờ", pending.len());
            }
            continue;
        }

        let skip = pending.len().saturating_sub(MAX_PENDING);
        if skip > 0 {
            eprintln!("[translate] bỏ {skip} câu tồn đọng (dịch chậm hơn phụ đề)");
        }
        for src in pending.into_iter().skip(skip) {
            if !running.load(Ordering::Relaxed) {
                break;
            }
            let out = translator.translate(&src);
            // Dịch tốn 200-800ms — người dùng có thể bấm dừng ngay trong lúc đó.
            // ponytail: vẫn còn khe hở vài micro giây giữa kiểm tra này và speak(); chấp nhận,
            // muốn kín hẳn thì phải đánh số thế hệ cho hàng đợi TTS.
            if !running.load(Ordering::Relaxed) {
                break;
            }
            // "" = AI vừa chạy lại được -> UI xoá cảnh báo.
            if let Some(notice) = translator.drain_notice() {
                let _ = app.emit("warn", notice);
            }
            *last_text.lock().unwrap() = out.clone();
            let _ = app.emit("text", Spoken { src, out: out.clone() });
            tts.speak(&out);
        }
    }
}
