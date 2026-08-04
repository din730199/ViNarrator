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
yarn test     # cargo test (44 unit test)
```

Installer ~2.8 MB, exe ~11 MB.

## Dùng

- **BẮT ĐẦU ĐỌC / DỪNG ĐỌC** — hoặc phím tắt toàn cục **Ctrl+Alt+S**. **Ctrl+Alt+R** đọc lại câu cuối.
- **SỬA VÙNG QUÉT** — kéo chuột chọn vùng phụ đề/thoại (overlay phủ màn hình chính).
- **VÙNG LOẠI TRỪ** — kéo chọn vùng con bỏ qua (HUD/logo) trong khung quét; nút *xoá* để bỏ.
- **CHẾ ĐỘ** — Anh→Việt hoặc Trung→Việt.
- **CÁCH DỊCH** — Google Dịch hoặc AI (xem dưới).
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

### Cách dịch

Mặc định là **Google Dịch** (endpoint web `client=gtx`, không cần key, ~150ms). Dịch từng câu
rời rạc nên hay sai xưng hô — "you" lúc thì *bạn*, lúc thì *mày*.

Chọn **AI** trong mục *CÁCH DỊCH* thì mỗi câu được gửi kèm vài câu trước làm ngữ cảnh, model
tự chọn xưng hô nhất quán và hiểu thuật ngữ game. Kéo thanh **NGỮ CẢNH** về 0 để tắt phần này
(rẻ token, nhanh hơn, dịch rời như Google).

Mọi provider đều gọi qua **một** API OpenAI-compatible (`POST {base_url}/chat/completions`),
chỉ khác `base_url` + `model`, nên thêm nhà cung cấp mới = sửa `LLM_PRESETS` trong `src/types.ts`.
Hạn mức free tier tra 8/2026:

| | Free tier | Trễ | Ghi chú |
|---|---|---|---|
| **Groq** (mặc định) | 30 lệnh/phút · 14.400/ngày | ~200–400ms | key ở [console.groq.com](https://console.groq.com), không cần thẻ |
| Gemini | 15/phút · **1.000/ngày** | ~500–900ms | dịch mượt nhất, nhưng hết quota sau ~1 giờ chơi |
| OpenRouter | model `:free` ~50/ngày | chậm, hay 429 | chỉ để thử |

ViNarrator quét 1,2s/lần, dedup lọc còn ~10–30 câu mới/phút — chỉ Groq chịu nổi liên tục.

Hỏng ở đâu cũng **không** làm gián đoạn giọng đọc: thiếu key / lỗi mạng / model trả lời dài
bất thường → tự lùi về Google Dịch; Google lỗi nữa → đọc nguyên văn. Dính **429 thì nghỉ hẳn
60s** rồi mới thử lại, gọi tiếp chỉ tổ ăn 429.

Lý do hỏng hiện **màu vàng ngay dưới thanh trạng thái** (vd `⚠ AI lỗi 401: Invalid API Key —
đang dùng Google Dịch`) và tự biến mất khi AI chạy lại. App là GUI nên không ai đọc được
stderr — không báo lên UI thì người dùng thấy vẫn đọc bình thường mà không biết AI đã ngừng.
Mỗi lần đổi tình trạng chỉ báo một lần, không spam từng câu.

API key lưu **thô** trong `%APPDATA%\ViNarrator\config.json` (`llm_api_key`) — app cá nhân,
không hardcode và không đóng gói kèm, nhưng đừng share file config đó.

### Chỉnh độ chính xác OCR

Ba knob đầu chỉnh ngay trong giao diện (**TINH CHỈNH BẮT CHỮ**), phần còn lại sửa trong `config.json`:

| Field | Mặc định | Ý nghĩa |
|---|---|---|
| `min_clean_ratio` | `0.6` | Bỏ dòng có điểm "sạch" chuẩn hoá dưới ngưỡng — chặn chữ OCR bịa. Nâng nếu còn nghe đọc rác, hạ (hoặc `0` để tắt) nếu câu thật bị bỏ oan. |
| `min_line_len` | `3` | Bỏ dòng ngắn hơn ngần này ký tự. Đừng lên 4: `Yes`, `No!`, `Go!`, `Run` đều đúng 3 ký tự. |
| `interval_ms` | `1200` | Nhịp quét. |
| `fuzzy_threshold` | `0.97` | Hai dòng giống nhau tới mức này thì bỏ dòng sau. **Đo được**, xem dưới. |
| `min_height_ratio` | `0.6` | Bỏ dòng thấp hơn `tỉ lệ × dòng cao nhất` trong khung. Cắt chữ nhỏ của giao diện (thanh tua, tiêu đề video). Đặt `0` để tắt. Áp cho cả hai engine. |
| `binarize_threshold` | `150` | Chỉ dùng cho Windows OCR: ngưỡng cho lượt OCR nhị phân hoá (`max(R,G,B) ≥ ngưỡng` = chữ). Hạ xuống nếu phụ đề mờ/xám; nâng lên nếu nền sáng bị nhận nhầm. |

**Lọc chữ rác** cho mỗi ký tự một điểm rồi lấy trung bình: chữ/số/khoảng trắng `+1`, ký tự lạ
`-3`, chữ có dấu **khi nguồn là tiếng Anh** `-6` (dấu tiếng Việt lọt vào khung tiếng Anh gần như
chắc chắn là OCR bịa). Dấu câu thường bị loại khỏi cả tử lẫn mẫu, nếu không `Hmm...` sẽ thành rác.
Đo thật: `' oanqCảaTôi Phả Server Minecraft Bi Bo H` được 0.50, câu đọc đúng được 1.00.
Chế độ Trung tắt luật chữ có dấu vì chữ Hán vốn ngoài ASCII.

Hai luật này **chồng lấn nhau**, biết ranh giới thì đỡ chỉnh nhầm: `min_clean_ratio` đã dọn sạch
rác dấu câu (`.` `-` `|` `'` `..` `•`), nên việc còn lại của `min_line_len` chỉ là mảnh **chữ cái**
1–2 ký tự (`o`, `ll`, `at`, `l1`). Vì vậy `3` là trần hợp lý — lên `4` là bắt đầu nuốt thoại thật.

Log hiện **cả bản gốc (mờ) lẫn bản dịch** — nghe sai thì nhìn là biết lỗi ở OCR hay ở khâu dịch.

### Ngưỡng chống đọc trùng — đo, không đoán

`fuzzy_threshold` từng là `0.82`, thừa hưởng từ bản Python dùng `difflib.SequenceMatcher`
(Ratcliff-Obershelp) rồi bê nguyên sang bản Rust dùng `normalized_levenshtein` — **hai metric
khác nhau, chưa hề đo lại**.

Đo trên 60 ảnh phụ đề xuống cấp (10 câu × 6 kiểu: chữ 9px trên nhiễu, tương phản cực thấp,
serif nghiêng, nền tần số cao, chữ mờ trên nền sáng), chạy qua `LineDeduper` thật:

| ngưỡng | đọc lặp (OneOCR) | mất câu thoại¹ |
|---|---|---|
| 0.82 (cũ) | 0 | **8/8** |
| 0.92 | 0 | 5/8 |
| 0.95 | 0 | 3/8 |
| **0.97 (nay)** | 1 | **0/8** |

¹ cặp thoại chỉ khác một từ — `"...touch that lever"` vs `"...that button"` (0.829),
`"more iron"` vs `"more gold"` (0.905), `"forty hours"` vs `"fifty hours"` (0.953).

Tức là ngưỡng cũ **sai theo chiều ngược với dự đoán**: không phải quá lỏng mà quá **gắt** —
nó nuốt sạch thoại gần giống nhau. Đổi lại chỉ được 0 lần đọc lặp.

Giới hạn thật của cách này: OCR đọc lệch giữa câu (`hits`→`bits`, 0.951) và thoại khác một từ
(`base`→`gate`, 0.950) **có điểm gần như nhau — không metric nào tách được**. Nên chọn ngưỡng
nghiêng về phía ĐỌC: nghe lặp một câu thì bực, mất một câu thì không lấy lại được.

`config.json` cũ ghi đúng `0.82` sẽ tự nâng lên khi nạp; ai tự chỉnh số khác thì giữ nguyên.

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
| `src-tauri/src/translate.rs` | Google Translate + LLM (OpenAI-compatible) | `translate.py` (deep-translator) |
| `src-tauri/src/tts.rs` | `Windows.Media.SpeechSynthesis` + `PlaySound` | `tts.py` (winsdk + winsound) |
| `src-tauri/src/dedup.rs` | chống đọc trùng phụ đề cuộn | `dedup.py` |¹
| `src-tauri/src/speech_queue.rs` | hàng đợi đọc (dedup + cap 3) | `speech_queue.py` |
| `src-tauri/src/narrator.rs` | 2 thread: chụp→OCR→dedup, và dịch→đọc | `narrator.py` |
| `src-tauri/src/config.rs` | đọc/ghi config | `config.py` |
| `src/App.tsx` | giao diện | `gui.py` |
| `src/overlay/Overlay.tsx` | overlay kéo chọn vùng | `region_select.py` |

¹ **Sửa một bug port từ bản Python:** luật "trừ hết phần đã đọc, còn ≤1 từ thì bỏ" chạy cả khi
không trừ được gì, nên câu **một từ** (`Run!`, `Help!`) luôn đếm ra 1 và bị tuyên trùng dù chưa
đọc bao giờ — đúng loại câu cảm thán hay gặp trong game. Nay chỉ áp luật khi thực sự có trừ.

**Dịch chạy ở thread riêng.** Một câu dịch bằng AI tốn 200–800ms; để chung vòng chụp thì khung
kế tiếp trễ đúng ngần ấy và phụ đề đổi trong lúc đó là mất luôn. Thread chụp đẩy câu qua channel,
thread dịch gom hết cái đã tới rồi **bỏ bớt, chỉ giữ 3 câu mới nhất** — dịch không kịp phụ đề thì
thà bỏ câu cũ còn hơn đọc trễ cả phút.

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
