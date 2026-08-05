mod capture;
mod config;
mod dedup;
mod narrator;
mod ocr;
mod oneocr;
mod piper;
mod speech_queue;
mod translate;
mod tts;

use std::cell::Cell;

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

/// engine: "windows" | "piper". Lần đầu chọn Piper phải tải ~82 MB — làm ở thread riêng và
/// báo qua event, KHÔNG chặn command: chặn ở đây thì UI đơ suốt thời gian tải.
/// Đổi config trước, tải sau: tải hỏng thì `tts.rs` tự lùi về giọng Windows, không câm.
#[tauri::command]
fn set_tts_engine(app: AppHandle, n: tauri::State<Narrator>, engine: String) -> Config {
    n.set_tts_engine(engine.clone());
    if engine == "piper" && !piper::ready() {
        std::thread::spawn(move || {
            let _ = app.emit("status", "Đang tải giọng Piper (~82 MB, chỉ lần đầu)...");
            match piper::ensure_runtime() {
                Ok(()) => {
                    let _ = app.emit("status", "Đã tải xong giọng Piper — sẵn sàng");
                    let _ = app.emit("warn", String::new());
                }
                Err(e) => {
                    let _ = app.emit(
                        "warn",
                        format!("Không tải được Piper: {e} — tạm đọc bằng giọng Windows"),
                    );
                }
            }
        });
    }
    n.config()
}

/// Đã có sẵn runtime Piper chưa — UI dùng để biết có phải cảnh báo "sẽ tải 82 MB" hay không.
#[tauri::command]
fn piper_ready() -> bool {
    piper::ready()
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
            set_tts_engine,
            piper_ready,
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
