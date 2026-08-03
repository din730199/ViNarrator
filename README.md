# ViNarrator — Thuyết minh game tiếng Việt (Tauri + React + Rust)

Chụp một vùng màn hình → OCR → dịch sang tiếng Việt → **đọc thành tiếng**.
Đóng gói thành **một file installer** (NSIS), không cần Python/venv/model tải thêm.

Kiến trúc bám theo RiTaiAI: logic nặng ở Rust (`src-tauri/src`), UI React/Vite (`src`), vỏ Tauri v2.

## Yêu cầu

- Windows 10/11
- Giọng tiếng Việt offline của Windows (vd **Microsoft An**).
  Chưa có: Settings → Time & Language → Speech → Manage voices → Add → Vietnamese.
- Để **build**: Rust 1.82+, `cargo-tauri`, Node 22+, Yarn.

## Chạy / build

```powershell
yarn install
yarn dev      # cargo tauri dev
yarn build    # cargo tauri build -> src-tauri/target/release/bundle/nsis/*-setup.exe
yarn test     # cargo test (24 unit test)
```

Installer ~2.8 MB, exe ~11 MB.

## Dùng

- **BẮT ĐẦU ĐỌC / DỪNG ĐỌC** — hoặc phím tắt toàn cục **Ctrl+Alt+S**. **Ctrl+Alt+R** đọc lại câu cuối.
- **SỬA VÙNG QUÉT** — kéo chuột chọn vùng phụ đề/thoại (overlay phủ màn hình chính).
- **VÙNG LOẠI TRỪ** — kéo chọn vùng con bỏ qua (HUD/logo) trong khung quét; nút *xoá* để bỏ.
- **CHẾ ĐỘ** — Anh→Việt hoặc Trung→Việt.
- **GIỌNG ĐỌC / ÂM LƯỢNG / TỐC ĐỘ**.

Cấu hình lưu ở `%APPDATA%\ViNarrator\config.json` (giữ nguyên tên field của bản Python
nên `config.json` cũ copy sang vẫn đọc được).

### Chỉnh độ chính xác OCR

Hai knob trong `config.json`, sửa khi phụ đề bị bỏ sót hoặc lẫn chữ rác:

| Field | Mặc định | Ý nghĩa |
|---|---|---|
| `min_height_ratio` | `0.6` | Bỏ dòng thấp hơn `tỉ lệ × dòng cao nhất` trong khung. Cắt chữ nhỏ của giao diện (thanh tua, tiêu đề video). Đặt `0` để tắt. |
| `binarize_threshold` | `150` | Ngưỡng cho lượt OCR nhị phân hoá (`max(R,G,B) ≥ ngưỡng` = chữ). Hạ xuống nếu phụ đề mờ/xám; nâng lên nếu nền sáng bị nhận nhầm. |

Mỗi khung được OCR **hai lượt** — ảnh gốc và ảnh nhị phân hoá — rồi lấy bản ít ký tự rác hơn.
Đo trên 8 mẫu (chữ nhỏ/lớn, nền tối/sáng, phụ đề màu trên nền ảnh động): chỉ dùng ảnh gốc
đạt trung bình 0.922 và tệ nhất 0.375; hai lượt + chọn đạt **1.000 ở cả 8 mẫu**.

## Kiến trúc

| File | Vai trò | Thay cho (bản Python) |
|------|---------|------------------------|
| `src-tauri/src/capture.rs` | chụp vùng màn hình BGRA + tô trắng vùng loại trừ | `capture.py`, `mask_exclusion` |
| `src-tauri/src/ocr.rs` | `Windows.Media.Ocr` | `ocr.py` (winsdk) |
| `src-tauri/src/translate.rs` | Google Translate qua `reqwest` | `translate.py` (deep-translator) |
| `src-tauri/src/tts.rs` | `Windows.Media.SpeechSynthesis` + `PlaySound` | `tts.py` (winsdk + winsound) |
| `src-tauri/src/dedup.rs` | chống đọc trùng phụ đề cuộn | `dedup.py` |
| `src-tauri/src/speech_queue.rs` | hàng đợi đọc (dedup + cap 3) | `speech_queue.py` |
| `src-tauri/src/narrator.rs` | worker chụp→OCR→dịch→đọc | `narrator.py` |
| `src-tauri/src/config.rs` | đọc/ghi config | `config.py` |
| `src/App.tsx` | giao diện | `gui.py` |
| `src/overlay/Overlay.tsx` | overlay kéo chọn vùng | `region_select.py` |

Unit test của bản Python (`tests/test_dedup.py`, `tests/test_speech_queue.py`) đã port nguyên
sang Rust và đều pass — dùng để kiểm chứng bản port.

## Khác biệt so với bản Python

Bỏ (kéo theo PyTorch vài GB, không có tương đương thuần Rust):

- **Chế độ "Đọc thuần" (OCR tiếng Việt bằng EasyOCR)** — Windows OCR không hỗ trợ tiếng Việt.
  Muốn có lại: nhúng PaddleOCR ONNX qua crate `ort`.
- **Giọng Piper** (`models/vi_VN-vais1000-medium.onnx`) — crate `piper-rs` kéo theo `espeak-rs-sys`,
  build cần **LLVM/libclang**. Cài `winget install LLVM.LLVM` rồi mới thêm được.
- **Giọng MMS** (`models/mms-tts-vie`) — đang là safetensors + torch; muốn chạy bằng `ort`
  phải export ONNX một lần rồi tự viết tokenizer ký tự từ `vocab.json`.

Mã Python cũ vẫn còn trong repo (`*.py`, `models/`) để đối chiếu; xoá được khi không cần nữa.
