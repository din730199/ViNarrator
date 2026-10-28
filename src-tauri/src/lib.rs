mod asr;
mod config;
mod loopback;
mod narrator;
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

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WindowEvent};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use config::Config;
use narrator::Narrator;

thread_local! {
    static COM_INIT: Cell<bool> = const { Cell::new(false) };
}

/// WinRT (TTS) đòi thread đã khởi tạo COM. Gọi đầu mỗi thread chạm WinRT — MTA, không uninit
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
    bat_tat(&app, &n)
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

/// Tải model nghe cho thứ tiếng `lang` ở thread riêng, báo tiến độ qua event — tải trong command
/// thì UI đơ suốt mấy phút. Đã đủ, hoặc đang có luồng khác tải, thì thôi.
fn tai_model(app: AppHandle, lang: String) {
    if asr::ready(&lang) {
        return;
    }
    let Some(khoa) = xin_cai(&DANG_CAI_ASR) else {
        return;
    };
    std::thread::spawn(move || {
        let _khoa = khoa;
        let bao = |s: String| {
            let _ = app.emit("status", s);
        };
        bao(format!(
            "Đang tải model nghe {} (~{} MB, chỉ lần đầu)…",
            translate::ten_tieng(&lang),
            asr::mb_can_tai(&lang)
        ));
        match asr::ensure_model(&lang, &bao) {
            Ok(()) => bao("Đã tải xong model nghe".into()),
            Err(e) => {
                let _ = app.emit("warn", format!("Không tải được model nghe: {e}"));
            }
        }
    });
}

/// engine: "local" | "speechmatics". Key để trong config của user, không hardcode.
/// Chọn bản chạy máy cho một thứ tiếng chưa có model thì tải luôn ở nền.
#[tauri::command]
fn set_asr(app: AppHandle, n: tauri::State<Narrator>, engine: String, key: String, language: String) -> Config {
    n.set_asr(engine.clone(), key, language.clone());
    if engine == "local" {
        tai_model(app, language);
    }
    n.config()
}

/// Model nghe thứ tiếng này đã tải chưa — UI dùng để biết có phải báo "sẽ tải" hay không.
#[tauri::command]
fn asr_ready(language: String) -> bool {
    asr::ready(&language)
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
fn set_volume(n: tauri::State<Narrator>, volume: u16) {
    n.set_volume(volume);
}

/// Bật/tắt cửa sổ phụ đề nổi. Không focus để khỏi giật game ra khỏi tiền cảnh; mặc định chuột
/// xuyên qua, chỉ `movable` mới bắt chuột để kéo.
fn apply_captions(app: &AppHandle, on: bool, movable: bool) {
    let Some(w) = app.get_webview_window("captions") else { return };
    if !on {
        let _ = w.hide();
        return;
    }
    // Vị trí chỉ đặt lúc mới hiện: giữa đáy màn hình chính, nơi phụ đề phim vẫn nằm.
    // shortcut: chưa nhớ vị trí người dùng đã kéo, mở lại là về chỗ cũ — lưu vào Config nếu cần.
    if !w.is_visible().unwrap_or(false) {
        if let (Ok(Some(m)), Ok(sz)) = (app.primary_monitor(), w.outer_size()) {
            let x = m.position().x + (m.size().width as i32 - sz.width as i32) / 2;
            let y = m.position().y + m.size().height as i32 - sz.height as i32 - (m.scale_factor() * 70.0) as i32;
            let _ = w.set_position(PhysicalPosition::new(x, y));
        }
        let _ = w.show();
    }
    let _ = w.set_always_on_top(true);
    let _ = w.set_ignore_cursor_events(!movable);
}

#[tauri::command]
fn set_captions(app: AppHandle, n: tauri::State<Narrator>, on: bool, movable: bool) {
    n.set_captions(on, movable);
    apply_captions(&app, on, movable);
    // Cửa sổ phụ đề đổi viền (nét đứt) theo cờ này.
    let _ = app.emit("captions_state", movable);
}

fn emit_running(app: &AppHandle, on: bool) -> bool {
    let _ = app.emit("running", on);
    on
}

/// Bật/tắt dùng chung cho nút trên UI **và** phím tắt Ctrl+Alt+S. Hai đường phải đi qua đúng
/// một chỗ, nếu không thì [`Narrator::preflight`] chỉ chặn được một nửa số cách bật.
fn bat_tat(app: &AppHandle, n: &Narrator) -> bool {
    // Chỉ kiểm lúc BẬT — tắt thì không có gì để hỏng, và không được phép chặn.
    if !n.running() {
        if let Err(e) = n.preflight() {
            let _ = app.emit("warn", e);
            return emit_running(app, false);
        }
        // Chưa có model thì vẫn bật: tải xong worker tự bắt đầu nghe, khỏi phải bấm lại.
        let c = n.config();
        if c.asr_engine == "local" {
            tai_model(app.clone(), c.asr_language);
        }
    }
    emit_running(app, n.toggle())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
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

            let c = app.state::<Narrator>().config();
            apply_captions(app.handle(), c.captions, c.captions_movable);

            // Hotkey toàn cục như bản headless: Ctrl+Alt+S bật/tắt, Ctrl+Alt+R đọc lại.
            // Đăng ký RIÊNG từng phím: gộp một lần thì một phím bị app khác chiếm sẽ làm hỏng cả hai.
            // Lỗi KHÔNG chặn app — vẫn dùng được nút trên UI.
            let gs = app.global_shortcut();
            let toggle_key = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyS);
            let reread_key = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyR);
            if let Err(e) = gs.on_shortcut(toggle_key, |app, _, event| {
                if event.state() == ShortcutState::Pressed {
                    bat_tat(app, &app.state::<Narrator>());
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
            // Cửa sổ phụ đề luôn tồn tại (ẩn) nên đóng cửa sổ chính KHÔNG tự thoát app -> thoát tay.
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
            set_translate,
            asr_ready,
            set_asr,
            set_tts_engine,
            engine_ready,
            vieneu_voices,
            set_voice,
            set_speed,
            set_volume,
            set_captions
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
