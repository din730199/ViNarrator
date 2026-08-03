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
yarn test     # cargo test (26 unit test)
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

### Engine OCR

Chọn trong giao diện (**ENGINE OCR**), đo bằng độ giống ground truth (0–1):

| | 8 mẫu dễ | 6 mẫu khó¹ | 3 mẫu tiếng Việt | ms/khung |
|---|---|---|---|---|
| **OneOCR** (mặc định) | 1.000 | **0.991** | **0.881** | ~70 |
| Windows OCR | 1.000 | 0.625 | 0.702 | ~20 |

¹ chữ 9px trên nhiễu, tương phản cực thấp, font nghiêng serif, nền tần số cao, chữ mờ trên nền sáng.

**OneOCR** là engine của Snipping Tool Win11. Ba file `oneocr.dll`, `oneocr.onemodel`,
`onnxruntime.dll` (~110 MB) thuộc Microsoft nên **không** đóng gói kèm — app tự chép từ gói
Snipping Tool trên máy vào cạnh exe ở lần chạy đầu (mất ~0.2s). Máy không có Snipping Tool
hoặc chạy Win10 → tự lùi về Windows OCR, lý do hiện ở thanh trạng thái.

Hai chi tiết kỹ thuật đáng nhớ:
- `build.rs` ép `/DELAYLOAD:oneocr.dll`. Không có nó, import tĩnh của `oneocr-rs` làm app chết
  ngay lúc nạp (`STATUS_DLL_NOT_FOUND`) trước khi kịp chạy code chép file.
- Phải hỏi vị trí gói qua WinRT `PackageManager`; user thường **không** liệt kê được
  `C:\Program Files\WindowsApps` (Access denied), dù đọc file theo đường dẫn chính xác thì được.
- `installMode` để `currentUser` (cài vào `%LOCALAPPDATA%`) nên chép file không cần quyền admin.
  Đổi sang `perMachine` sẽ làm OneOCR không dùng được.

### Chỉnh độ chính xác OCR

Hai knob trong `config.json`, sửa khi phụ đề bị bỏ sót hoặc lẫn chữ rác:

| Field | Mặc định | Ý nghĩa |
|---|---|---|
| `min_height_ratio` | `0.6` | Bỏ dòng thấp hơn `tỉ lệ × dòng cao nhất` trong khung. Cắt chữ nhỏ của giao diện (thanh tua, tiêu đề video). Đặt `0` để tắt. Áp cho cả hai engine. |
| `binarize_threshold` | `150` | Chỉ dùng cho Windows OCR: ngưỡng cho lượt OCR nhị phân hoá (`max(R,G,B) ≥ ngưỡng` = chữ). Hạ xuống nếu phụ đề mờ/xám; nâng lên nếu nền sáng bị nhận nhầm. |

Nhánh Windows OCR chạy **hai lượt** — ảnh gốc và ảnh nhị phân hoá — rồi lấy bản ít ký tự rác hơn:
chỉ ảnh gốc đạt trung bình 0.922 / tệ nhất 0.375 trên 8 mẫu dễ, hai lượt đạt 1.000 cả 8.
OneOCR không cần trò này.

Confidence của OneOCR **không** dùng để lọc rác được: chữ nhỏ của giao diện cũng được đọc đúng
(~0.99) trong khi tiếng Việt đọc đúng lại chỉ 0.72 — hai dải chồng nhau. Lọc theo chiều cao dòng
mới là cái phân biệt được.

## Kiến trúc

| File | Vai trò | Port từ (bản Python) |
|------|---------|------------------------|
| `src-tauri/src/capture.rs` | chụp vùng màn hình BGRA + tô trắng vùng loại trừ | `capture.py`, `mask_exclusion` |
| `src-tauri/src/ocr.rs` | chọn engine + `Windows.Media.Ocr` | `ocr.py` (winsdk) |
| `src-tauri/src/oneocr.rs` | OneOCR (Snipping Tool) + tự chép runtime | — (mới) |
| `src-tauri/src/translate.rs` | Google Translate qua `reqwest` | `translate.py` (deep-translator) |
| `src-tauri/src/tts.rs` | `Windows.Media.SpeechSynthesis` + `PlaySound` | `tts.py` (winsdk + winsound) |
| `src-tauri/src/dedup.rs` | chống đọc trùng phụ đề cuộn | `dedup.py` |
| `src-tauri/src/speech_queue.rs` | hàng đợi đọc (dedup + cap 3) | `speech_queue.py` |
| `src-tauri/src/narrator.rs` | worker chụp→OCR→dịch→đọc | `narrator.py` |
| `src-tauri/src/config.rs` | đọc/ghi config | `config.py` |
| `src/App.tsx` | giao diện | `gui.py` |
| `src/overlay/Overlay.tsx` | overlay kéo chọn vùng | `region_select.py` |

Unit test của bản Python (`tests/test_dedup.py`, `tests/test_speech_queue.py`) đã port nguyên
sang Rust và đều pass — dùng để kiểm chứng bản port. Cột "port từ" trỏ tới file của bản Python,
xem được ở commit `64e09eb`.

## Bản Python cũ

Đã xoá khỏi cây làm việc. Cần đối chiếu thì lấy từ git:

```bash
git show 64e09eb:gui.py          # xem 1 file
git checkout 64e09eb -- narrator.py ocr.py tts.py   # lấy lại vài file
```

Ba thứ của bản Python **chưa có** ở bản Rust (đều kéo theo PyTorch vài GB nên bỏ):

| Bỏ | Vì sao | Muốn có lại |
|---|---|---|
| Chế độ "Đọc thuần" (OCR tiếng Việt, EasyOCR) | Windows OCR không hỗ trợ tiếng Việt | nhúng PaddleOCR ONNX qua crate `ort` |
| Giọng Piper | `piper-rs` → `espeak-rs-sys`, build cần LLVM/libclang | `winget install LLVM.LLVM` rồi thêm crate |
| Giọng MMS | đang là safetensors + torch | export ONNX một lần + viết tokenizer ký tự từ `vocab.json` |

Thư mục `models/` (~200 MB) cũng đã xoá và nằm trong `.gitignore` — `model.safetensors` 145 MB
vượt giới hạn 100 MB của GitHub. Tải lại khi cần:

- Piper `vi_VN-vais1000-medium.onnx` (+ `.onnx.json`) —
  [rhasspy/piper-voices](https://huggingface.co/rhasspy/piper-voices/tree/main/vi/vi_VN)
- MMS `mms-tts-vie` — [Xenova/mms-tts-vie](https://huggingface.co/Xenova/mms-tts-vie)
