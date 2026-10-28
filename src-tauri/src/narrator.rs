//! Narrator — worker thread: nghe tiếng game -> dịch sang tiếng Việt -> đọc.
//! Trạng thái đẩy về UI qua event Tauri: "status" (chuỗi trạng thái) và "text" (câu vừa đọc).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use crate::config::{self, Config};
use crate::translate::{self, Translator};
use crate::tts::TtsEngine;

/// Cờ dựng lại từng phần.
#[derive(Default)]
struct Reload {
    translate: AtomicBool,
    /// Cấu hình nghe đổi -> thả cái tai đang chạy để nó dựng lại với key/ngôn ngữ mới.
    asr: AtomicBool,
}

impl Reload {
    fn all() -> Self {
        Self { translate: AtomicBool::new(true), asr: AtomicBool::new(false) }
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

    /// Kiểm những thứ hay hỏng nhất **trước** khi bật, đồng bộ, để báo ngay thay vì bật lên rồi
    /// ngồi im không nghe gì.
    pub fn preflight(&self) -> Result<(), String> {
        let c = self.cfg.lock().unwrap();
        if c.asr_engine == "speechmatics" {
            if c.speechmatics_key.trim().is_empty() {
                return Err("Chưa có API key Speechmatics — điền key, hoặc đổi sang bản chạy máy."
                    .into());
            }
            return Ok(());
        }
        // Bản chạy máy chưa có model: KHÔNG chặn — lib.rs tải ở nền, xong worker tự nghe.
        Ok(())
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

    /// Đổi cách nghe. Cái tai đang chạy phải dựng lại thì mới ăn cấu hình mới — hạ cờ
    /// `running` một nhịp là vòng lặp worker tự thả `Tai` cũ rồi bật cái mới.
    pub fn set_asr(&self, engine: String, key: String, language: String) {
        self.edit(|c| {
            c.asr_engine = engine.clone();
            c.speechmatics_key = key.clone();
            c.asr_language = language.clone();
        });
        self.reload.asr.store(true, Ordering::Relaxed);
        // Ngôn ngữ nguồn của khâu dịch đi theo tiếng đang nghe.
        self.reload.translate.store(true, Ordering::Relaxed);
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

    /// engine: "windows" | "vieneu".
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

    pub fn set_volume(&self, volume: u16) {
        self.edit(|c| c.volume = volume);
        self.tts.set_params(None, None, None, Some(volume));
    }

    pub fn set_captions(&self, on: bool, movable: bool) {
        self.edit(|c| {
            c.captions = on;
            c.captions_movable = movable;
        });
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
        // Dịch + đọc chạy ở thread RIÊNG: một câu dịch bằng AI tốn 200-800ms, để chung thread
        // nghe thì tiếng tới trong lúc đó dồn lại.
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

        // Cái tai sống trong thread RIÊNG: `cpal::Stream` và stream của sherpa đều `!Send`, phải
        // dựng và chết trong cùng một thread.
        let mut tai: Option<Tai> = None;

        loop {
            if self.quit.load(Ordering::Relaxed) {
                tai.take();
                return;
            }
            let cfg = self.cfg.lock().unwrap().clone();

            if self.reload.translate.swap(false, Ordering::Relaxed) {
                let _ = tx.send(Msg::Reload(Box::new(cfg.clone())));
                if tai.is_none() {
                    self.status(format!("Sẵn sàng — {}", translate::describe(&cfg)));
                }
            }

            // Speechmatics chạy trên mạng nên KHÔNG cần model tải về; chỉ bản chạy máy mới cần.
            // Chưa tải xong thì chờ — tải xong vòng sau tự bật, không phải bấm lại.
            let can_tai = self.running.load(Ordering::Relaxed)
                && (cfg.asr_engine == "speechmatics" || crate::asr::ready(&cfg.asr_language));
            // Đổi engine/key/ngôn ngữ giữa chừng -> thả cái tai cũ, nhánh dưới bật lại ngay.
            if self.reload.asr.swap(false, Ordering::Relaxed) {
                tai.take();
            }
            match (can_tai, tai.is_some()) {
                (true, false) => {
                    tai = Some(Tai::bat(&cfg, tx.clone(), self.tts.dang_doc(), self.app.clone()));
                }
                (false, true) => {
                    tai.take();
                    self.status("Đã dừng nghe");
                }
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Cái tai: một thread chạy [`crate::asr::run`]. Thả `Tai` là ra hiệu dừng rồi CHỜ thread ra
/// hẳn — phải chờ thật, vì `asr::run` còn chốt nốt câu dở trước khi thoát, và vì không chờ thì
/// đổi nguồn qua lại nhanh sẽ có hai thread cùng giữ loopback.
struct Tai {
    stop: Arc<AtomicBool>,
    tay: Option<std::thread::JoinHandle<()>>,
}

impl Tai {
    fn bat(cfg: &Config, tx: Sender<Msg>, dang_doc: Arc<AtomicBool>, app: AppHandle) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let (engine, key, ngon_ngu) = (
            cfg.asr_engine.clone(),
            cfg.speechmatics_key.clone(),
            cfg.asr_language.clone(),
        );
        let tay = std::thread::spawn(move || {
            let ra = |cau: String| {
                let _ = tx.send(Msg::Line(cau));
            };
            let bao = |m: String| {
                let _ = app.emit("status", m);
            };
            if let Err(e) = crate::asr::run(&engine, &key, &ngon_ngu, &s, &dang_doc, &ra, &bao) {
                let _ = app.emit("warn", format!("Không nghe được tiếng game: {e}"));
            }
        });
        Self { stop, tay: Some(tay) }
    }
}

impl Drop for Tai {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.tay.take() {
            let _ = t.join();
        }
    }
}

enum Msg {
    Line(String),
    Reload(Box<Config>),
}

/// Câu tồn đọng tối đa. Dịch không kịp phụ đề thì thà bỏ câu cũ còn hơn đọc trễ cả phút —
/// cùng lý do hàng đợi TTS cap 3.
const MAX_PENDING: usize = 3;

/// Một mục log: giữ cả bản gốc để phân biệt nghe sai với dịch sai.
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
