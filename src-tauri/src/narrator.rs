//! Narrator — worker thread: chụp -> (mask loại trừ) -> OCR -> (dịch) -> đọc. Port từ narrator.py.
//! Trạng thái đẩy về UI qua event Tauri: "status" (chuỗi trạng thái) và "text" (câu vừa đọc).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use crate::capture;
use crate::config::{self, Config, Region};
use crate::dedup::LineDeduper;
use crate::ocr::Ocr;
use crate::translate::Translator;
use crate::tts::TtsEngine;

pub struct Narrator {
    cfg: Arc<Mutex<Config>>,
    running: Arc<AtomicBool>,
    quit: Arc<AtomicBool>,
    reload_ocr: Arc<AtomicBool>,
    last_text: Arc<Mutex<String>>,
    tts: Arc<TtsEngine>,
}

impl Narrator {
    pub fn new(app: AppHandle, cfg: Config) -> Self {
        let tts = Arc::new(TtsEngine::new(cfg.voice_name.clone(), cfg.speed, cfg.volume));
        let me = Self {
            cfg: Arc::new(Mutex::new(cfg)),
            running: Arc::new(AtomicBool::new(false)),
            quit: Arc::new(AtomicBool::new(false)),
            reload_ocr: Arc::new(AtomicBool::new(true)),
            last_text: Arc::new(Mutex::new(String::new())),
            tts,
        };
        let w = Worker {
            app,
            cfg: me.cfg.clone(),
            running: me.running.clone(),
            quit: me.quit.clone(),
            reload_ocr: me.reload_ocr.clone(),
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

    pub fn set_mode(&self, mode: &str) {
        self.edit(|c| c.apply_mode(mode));
        self.reload_ocr.store(true, Ordering::Relaxed);
    }

    pub fn set_engine(&self, engine: String) {
        self.edit(|c| c.ocr_engine = engine.clone());
        self.reload_ocr.store(true, Ordering::Relaxed);
    }

    pub fn set_voice(&self, name: String) {
        self.edit(|c| c.voice_name = name.clone());
        self.tts.set_params(Some(name), None, None);
    }

    pub fn set_speed(&self, speed: f64) {
        self.edit(|c| c.speed = speed);
        self.tts.set_params(None, Some(speed), None);
    }

    pub fn set_volume(&self, volume: u8) {
        self.edit(|c| c.volume = volume);
        self.tts.set_params(None, None, Some(volume));
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
    reload_ocr: Arc<AtomicBool>,
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
        let mut translator = Translator::new(false, "en", "vi");
        let mut deduper = {
            let c = self.cfg.lock().unwrap();
            LineDeduper::new(c.dedup_require_stable, c.min_line_len, c.fuzzy_threshold)
        };

        loop {
            if self.quit.load(Ordering::Relaxed) {
                return;
            }
            let cfg = self.cfg.lock().unwrap().clone();

            if self.reload_ocr.swap(false, Ordering::Relaxed) {
                // Lần đầu chọn OneOCR phải chép ~110MB từ gói Snipping Tool -> báo trước
                // để người dùng không tưởng app treo.
                if cfg.ocr_engine != "windows" && !crate::oneocr::runtime_ready() {
                    self.status("Đang chép engine OneOCR từ Snipping Tool (~110 MB, chỉ lần đầu)...");
                } else {
                    self.status("Đang khởi tạo OCR...");
                }
                match Ocr::new(&cfg) {
                    Ok(o) => {
                        translator =
                            Translator::new(cfg.translate, &cfg.translate_from, &cfg.translate_to);
                        self.status(format!(
                            "Sẵn sàng — {}, dịch {}",
                            o.info(),
                            if translator.enabled { "ON" } else { "OFF" }
                        ));
                        ocr = Some(o);
                    }
                    Err(e) => {
                        self.status(format!("Lỗi khởi tạo OCR: {e}"));
                        ocr = None;
                    }
                }
                deduper.reset();
            }

            let Some(engine) = ocr.as_ref().filter(|_| self.running.load(Ordering::Relaxed)) else {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            };

            let t0 = Instant::now();
            if let Err(e) = self.tick(engine, &translator, &mut deduper, &cfg) {
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
        translator: &Translator,
        deduper: &mut LineDeduper,
        cfg: &Config,
    ) -> Result<(), String> {
        let (mut rgba, w, h) = capture::grab_rgba(&cfg.region)?;
        capture::mask_exclusion(&mut rgba, w, h, &cfg.region, cfg.exclude_region.as_ref());
        let text = engine.recognize_rgba(&rgba, w, h, cfg)?;
        for line in deduper.feed(&text) {
            let spoken = translator.translate(&line);
            *self.last_text.lock().unwrap() = spoken.clone();
            let _ = self.app.emit("text", &spoken);
            self.tts.speak(&spoken);
        }
        Ok(())
    }
}
