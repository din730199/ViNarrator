mod asr;
mod capture;
mod config;
mod dedup;
mod loopback;
mod narrator;
mod ocr;
mod oneocr;
mod runtime;
mod speech_queue;
mod speechmatics;
mod stretch;
mod translate;
mod tts;
mod vieneu;
mod wave_out;

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WindowEvent};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use config::{Config, Region};
use narrator::Narrator;

thread_local! {
    static COM_INIT: Cell<bool> = const { Cell::new(false) };
}

/// WinRT (OCR/TTS) đòi thread đã khởi tạo COM. Gọi đầu mỗi thread chạm WinRT — MTA, không uninit
/// (giữ apartment sống suốt đời thread; uninit sớm sẽ làm hỏng object đang cầm).
pub(crate) fn ensure_com() {
    COM_INIT.with(|done| {
        if !done.get() {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            done.set(true);
        }
    });
}

#[tauri::command]
fn get_config(n: tauri::State<Narrator>) -> Config {
    n.config()
}

/// async = chạy ngoài main thread. Liệt kê giọng WinRT lần đầu tốn cả giây; để sync sẽ chặn
/// luồng UI và cửa sổ kẹt ở "Đang tải…" cho tới khi xong.
#[tauri::command]
async fn list_voices() -> Vec<tts::Voice> {
    tts::list_voices()
}

#[tauri::command]
fn is_running(n: tauri::State<Narrator>) -> bool {
    n.running()
}

#[tauri::command]
fn toggle(app: AppHandle, n: tauri::State<Narrator>) -> bool {
    emit_running(&app, n.toggle())
}

#[tauri::command]
fn set_mode(n: tauri::State<Narrator>, mode: String) -> Config {
    n.set_mode(&mode);
    n.config()
}

/// engine: "oneocr" | "windows". Đổi xong worker dựng lại engine ở vòng lặp kế tiếp.
#[tauri::command]
fn set_engine(n: tauri::State<Narrator>, engine: String) -> Config {
    n.set_engine(engine);
    n.config()
}

/// engine: "google" | "llm". Ba field LLM chỉ dùng khi engine = "llm".
#[tauri::command]
fn set_translate(
    n: tauri::State<Narrator>,
    engine: String,
    base_url: String,
    model: String,
    api_key: String,
    context_lines: usize,
) -> Config {
    n.set_translate(engine, base_url, model, api_key, context_lines);
    n.config()
}

#[tauri::command]
fn set_tuning(
    n: tauri::State<Narrator>,
    interval_ms: u64,
    min_line_len: usize,
    min_clean_ratio: f32,
    fuzzy_threshold: f64,
) -> Config {
    n.set_tuning(interval_ms, min_line_len, min_clean_ratio, fuzzy_threshold);
    n.config()
}

/// engine: "windows" | "vieneu". Lần đầu chọn VieNeu phải cài ~560 MB — làm ở thread riêng và
/// báo qua event, KHÔNG chặn command: chặn ở đây thì UI đơ suốt thời gian cài.
/// Đổi config trước, cài sau: cài hỏng thì `tts.rs` tự lùi về giọng Windows.
static DANG_CAI_TTS: AtomicBool = AtomicBool::new(false);
static DANG_CAI_ASR: AtomicBool = AtomicBool::new(false);

/// Thả khoá kể cả khi luồng cài panic — khoá kẹt nghĩa là tới khi khởi động lại app mới cài
/// lại được, mà lúc đó `ready()` vẫn false nên người dùng kẹt luôn ở lựa chọn cũ.
struct KhoaCai(&'static AtomicBool);

impl Drop for KhoaCai {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Xin quyền chạy một luồng cài. `None` = đã có luồng khác đang cài đúng thứ đó.
///
/// Không có khoá này thì bấm VieNeu → Windows → VieNeu lúc đang cài sẽ dựng luồng thứ hai
/// (`ready()` vẫn false suốt quá trình cài): hai `uv pip install` cùng ghi vào một venv, hai
/// lượt kéo 285 MB model vào cùng cache, và tệ nhất là `VieNeu::start_timeout` của luồng sau
/// `remove_dir_all(out)` **xoá luôn thư mục mẩu tiếng** mà luồng trước đang dùng.
///
/// Hai cờ RIÊNG cho TTS và ASR: dùng chung một cờ thì đang tải model nghe mà bấm chọn VieNeu
/// sẽ bị nuốt lặng lẽ.
fn xin_cai(co: &'static AtomicBool) -> Option<KhoaCai> {
    (!co.swap(true, Ordering::SeqCst)).then(|| KhoaCai(co))
}

/// source: "screen" | "audio". Lần đầu chọn "audio" phải tải model ~198 MB — tải ở thread riêng
/// như VieNeu. Đổi config trước, tải sau: chưa tải xong thì `narrator` vẫn chạy đường OCR.
#[tauri::command]
fn set_source(app: AppHandle, n: tauri::State<Narrator>, source: String) -> Config {
    n.set_source(source.clone());
    if source != "audio" || asr::ready() {
        return n.config();
    }
    let Some(khoa) = xin_cai(&DANG_CAI_ASR) else {
        return n.config();
    };
    std::thread::spawn(move || {
        let _khoa = khoa;
        let bao = |s: String| {
            let _ = app.emit("status", s);
        };
        bao("Đang tải model nghe tiếng Anh (~198 MB, chỉ lần đầu)…".into());
        match asr::ensure_model(&bao) {
            Ok(()) => bao("Đã tải xong — bật công tắc để bắt đầu nghe".into()),
            Err(e) => {
                let _ = app.emit(
                    "warn",
                    format!("Không tải được model nghe: {e} — tạm đọc phụ đề trên màn hình"),
                );
            }
        }
    });
    n.config()
}

/// engine: "local" | "speechmatics". Key để trong config của user, không hardcode.
#[tauri::command]
fn set_asr(n: tauri::State<Narrator>, engine: String, key: String, language: String) -> Config {
    n.set_asr(engine, key, language);
    n.config()
}

/// Model nghe đã tải chưa — UI dùng để biết có phải báo "sẽ tải 198 MB" hay không.
#[tauri::command]
fn asr_ready() -> bool {
    asr::ready()
}

#[tauri::command]
fn set_tts_engine(app: AppHandle, n: tauri::State<Narrator>, engine: String) -> Config {
    n.set_tts_engine(engine.clone());
    if engine != "vieneu" || vieneu::ready() {
        return n.config();
    }
    let Some(khoa) = xin_cai(&DANG_CAI_TTS) else {
        return n.config();
    };
    std::thread::spawn(move || {
        let _khoa = khoa;
        // Tiến độ cài đi thẳng ra thanh trạng thái: 560 MB mà thanh đứng im thì người dùng
        // tưởng app treo.
        let bao = |s: String| {
            let _ = app.emit("status", s);
        };
        // 561 MB trên đĩa: uv 46 + CPython 60 + gói 163 + model 285.
        bao("Đang cài giọng VieNeu (~560 MB, chỉ lần đầu)…".into());
        match vieneu::ensure_runtime(&bao) {
            Ok(()) => {
                bao("Đã cài xong giọng VieNeu — đang chuẩn bị…".into());
                let _ = app.emit("warn", String::new());
                // Dựng sẵn engine ngay, đừng để câu đọc đầu tiên gánh ~10s nạp model.
                app.state::<Narrator>().prewarm();
            }
            Err(e) => {
                let _ = app.emit(
                    "warn",
                    format!("Không cài được VieNeu: {e} — tạm đọc bằng giọng Windows"),
                );
            }
        }
    });
    n.config()
}

/// 14 giọng của VieNeu, lấy từ chính model (worker ghi ra lúc cài) chứ không chép cứng ở UI.
#[tauri::command]
fn vieneu_voices() -> Vec<String> {
    vieneu::voices()
}

/// Runtime VieNeu đã cài sẵn chưa — UI dùng để biết có phải báo "sẽ tải 560 MB" hay không.
#[tauri::command]
fn engine_ready(engine: String) -> bool {
    engine != "vieneu" || vieneu::ready()
}

#[tauri::command]
fn set_voice(n: tauri::State<Narrator>, name: String) {
    n.set_voice(name);
}

#[tauri::command]
fn set_speed(n: tauri::State<Narrator>, speed: f64) {
    n.set_speed(speed);
}

#[tauri::command]
fn set_volume(n: tauri::State<Narrator>, volume: u8) {
    n.set_volume(volume);
}

/// Mở overlay kéo chọn vùng. kind: "scan" (vùng quét) | "exclude" (vùng loại trừ).
#[tauri::command]
fn pick_region(app: AppHandle, kind: String) -> Result<(), String> {
    let overlay = app
        .get_webview_window("overlay")
        .ok_or("không có cửa sổ overlay")?;
    // Phủ đúng màn hình chính (giống overlay fullscreen của tkinter).
    if let Ok(Some(m)) = app.primary_monitor() {
        let _ = overlay.set_position(PhysicalPosition::new(m.position().x, m.position().y));
        let _ = overlay.set_size(PhysicalSize::new(m.size().width, m.size().height));
    }
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.hide(); // ẩn cửa sổ chính để chọn được vùng nằm dưới nó
    }
    overlay.show().map_err(|e| e.to_string())?;
    let _ = overlay.set_always_on_top(true);
    let _ = overlay.set_focus();
    let _ = app.emit_to("overlay", "pick_start", kind);
    Ok(())
}

/// Overlay gọi lại khi kéo xong (region = None nếu bấm Esc).
/// Command này chạy từ cửa sổ overlay nên giá trị trả về KHÔNG tới được cửa sổ chính —
/// phải emit "config" thì UI chính mới hiện vùng mới.
#[tauri::command]
fn region_picked(app: AppHandle, n: tauri::State<Narrator>, kind: String, region: Option<Region>) {
    if let Some(overlay) = app.get_webview_window("overlay") {
        let _ = overlay.hide();
    }
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
    }
    if let Some(r) = region {
        if kind == "exclude" {
            n.set_exclude(Some(r));
        } else {
            n.set_region(r);
        }
        let _ = app.emit("config", n.config());
    }
}

/// Xoá vùng loại trừ (quét lại toàn khung).
#[tauri::command]
fn clear_exclude(n: tauri::State<Narrator>) -> Config {
    n.set_exclude(None);
    n.config()
}

fn emit_running(app: &AppHandle, on: bool) -> bool {
    let _ = app.emit("running", on);
    on
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let handle = app.handle().clone();
            app.manage(Narrator::new(handle.clone(), config::load()));

            // Tauri tạo cửa sổ khai trong config TRƯỚC khi chạy setup -> webview có thể gọi
            // get_config lúc Narrator chưa manage => "state not managed", UI kẹt ở "Đang tải…".
            // Nên đặt "create": false rồi tự dựng cửa sổ Ở ĐÂY, sau khi state đã sẵn sàng.
            let window_configs = app.config().app.windows.clone();
            for wc in &window_configs {
                tauri::WebviewWindowBuilder::from_config(app.handle(), wc)?.build()?;
            }

            // Hotkey toàn cục như bản headless: Ctrl+Alt+S bật/tắt, Ctrl+Alt+R đọc lại.
            // Đăng ký RIÊNG từng phím: gộp một lần thì một phím bị app khác chiếm sẽ làm hỏng cả hai.
            // Lỗi KHÔNG chặn app — vẫn dùng được nút trên UI.
            let gs = app.global_shortcut();
            let toggle_key = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyS);
            let reread_key = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyR);
            if let Err(e) = gs.on_shortcut(toggle_key, |app, _, event| {
                if event.state() == ShortcutState::Pressed {
                    emit_running(app, app.state::<Narrator>().toggle());
                }
            }) {
                eprintln!("[hotkey] Ctrl+Alt+S không đăng ký được: {e}");
            }
            if let Err(e) = gs.on_shortcut(reread_key, |app, _, event| {
                if event.state() == ShortcutState::Pressed {
                    app.state::<Narrator>().reread();
                }
            }) {
                eprintln!("[hotkey] Ctrl+Alt+R không đăng ký được: {e}");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Overlay luôn tồn tại (ẩn) nên đóng cửa sổ chính KHÔNG tự thoát app -> thoát tay.
            if window.label() == "main" && matches!(event, WindowEvent::CloseRequested { .. }) {
                window.app_handle().state::<Narrator>().shutdown();
                window.app_handle().exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            list_voices,
            is_running,
            toggle,
            set_mode,
            set_engine,
            set_translate,
            set_tuning,
            set_source,
            asr_ready,
            set_asr,
            set_tts_engine,
            engine_ready,
            vieneu_voices,
            set_voice,
            set_speed,
            set_volume,
            pick_region,
            region_picked,
            clear_exclude
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bấm chọn VieNeu lần thứ hai lúc lần cài đầu chưa xong thì KHÔNG được dựng luồng cài
    /// thứ hai — xem `xin_cai`. Và cài xong phải thả khoá, nếu không sẽ không bao giờ cài lại.
    #[test]
    fn khong_cai_chong_hai_luong() {
        let khoa = xin_cai(&DANG_CAI_TTS).expect("luồng cài đầu tiên phải xin được khoá");
        assert!(xin_cai(&DANG_CAI_TTS).is_none(), "đang cài dở mà vẫn cho luồng thứ hai chạy");
        // Khoá của TTS KHÔNG được chặn luồng tải model nghe — hai thứ khác nhau.
        let k2 = xin_cai(&DANG_CAI_ASR).expect("khoá ASR bị khoá TTS chặn oan");
        drop(khoa);
        drop(k2);
        assert!(xin_cai(&DANG_CAI_TTS).is_some(), "cài xong không thả khoá — kẹt luôn ở lựa chọn cũ");
        assert!(xin_cai(&DANG_CAI_ASR).is_some());
    }
}
