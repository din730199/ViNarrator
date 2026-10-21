# ViNarrator — Thuyết minh game tiếng Việt (Tauri + React + Rust)

Chụp một vùng màn hình → OCR → dịch sang tiếng Việt → **đọc thành tiếng**.
Đóng gói thành **một file installer** (NSIS), không cần Python/venv. Cài xong chạy được ngay;
giọng neural VieNeu là tuỳ chọn, bật thì mới cài thêm.

Kiến trúc bám theo RiTaiAI: logic nặng ở Rust (`src-tauri/src`), UI React/Vite (`src`), vỏ Tauri v2.

## Yêu cầu

- Windows 10/11
- Giọng tiếng Việt offline của Windows (vd **Microsoft An**).
  Chưa có: Settings → Time & Language → Speech → Manage voices → Add → Vietnamese.
  (Hoặc bật giọng **VieNeu** trong app — không cần cài gì vào Windows, xem *Giọng đọc*.)
- Giọng **VieNeu** cần Microsoft Visual C++ Redistributable x64 mới. Máy Win11 sạch có thể báo
  `exit code: 0xc0000005` lúc cài — tải https://aka.ms/vs/17/release/vc_redist.x64.exe, cài rồi bật lại.
- Để **build**: Rust 1.82+, `cargo-tauri`, Node 22+, Yarn.

## Chạy / build

```powershell
yarn install
yarn dev      # cargo tauri dev
yarn build    # cargo tauri build -> <target>/release/bundle/nsis/*-setup.exe
              # <target> = %CARGO_TARGET_DIR% nếu đã set, mặc định src-tauri/target
yarn test     # cargo test (52 unit test)
```

Installer ~3,4 MB, exe ~13,6 MB — giọng neural tải riêng khi bật, không nhét vào installer.

Năm test đụng mạng/loa nên để `#[ignore]`, chạy tay khi sửa phần giọng đọc:

```powershell
cargo test --lib -- --ignored --nocapture --test-threads=1
```

## Dùng

- **BẮT ĐẦU ĐỌC / DỪNG ĐỌC** — hoặc phím tắt toàn cục **Ctrl+Alt+S**. **Ctrl+Alt+R** đọc lại câu cuối.
- **SỬA VÙNG QUÉT** — kéo chuột chọn vùng phụ đề/thoại (overlay phủ màn hình chính).
- **VÙNG LOẠI TRỪ** — kéo chọn vùng con bỏ qua (HUD/logo) trong khung quét; nút *xoá* để bỏ.
- **CHẾ ĐỘ** — Anh→Việt hoặc Trung→Việt.
- **CÁCH DỊCH** — Google Dịch hoặc AI (xem dưới).
- **GIỌNG ĐỌC** — giọng Windows hoặc VieNeu (xem dưới) — **/ ÂM LƯỢNG / TỐC ĐỘ**.

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
| `interval_ms` | `700` | Nhịp quét. Đừng nâng cao: nhân với `dedup_require_stable` thành thời gian phụ đề phải đứng yên mới bắt được. |
| `fuzzy_threshold` | `0.97` | Hai dòng giống nhau tới mức này thì bỏ dòng sau. **Đo được**, xem dưới. |
| `min_height_ratio` | `0.6` | Bỏ dòng thấp hơn `tỉ lệ × dòng cao nhất` trong khung. Cắt chữ nhỏ của giao diện (thanh tua, tiêu đề video). Đặt `0` để tắt. Áp cho cả hai engine. |
| `binarize_threshold` | `150` | Chỉ dùng cho Windows OCR: ngưỡng cho lượt OCR nhị phân hoá (`max(R,G,B) ≥ ngưỡng` = chữ). Hạ xuống nếu phụ đề mờ/xám; nâng lên nếu nền sáng bị nhận nhầm. |

**Lọc chữ rác** cho mỗi ký tự một điểm rồi lấy trung bình: chữ/số/khoảng trắng `+1`, ký tự lạ
`-3`, chữ có dấu **khi nguồn là tiếng Anh** `-6` (dấu tiếng Việt lọt vào khung tiếng Anh gần như
chắc chắn là OCR bịa). Dấu câu thường bị loại khỏi cả tử lẫn mẫu, nếu không `Hmm...` sẽ thành rác.
Đo thật: `' oanqCảaTôi Phả Server Minecraft Bi Bo H` được 0.50, câu đọc đúng được 1.00.
Chế độ Trung tắt luật chữ có dấu vì chữ Hán vốn ngoài ASCII.

**Nhịp quét nhân đôi thành độ trễ.** `dedup_require_stable` bắt một dòng phải xuất hiện ở **hai
khung liên tiếp** mới được đọc (tránh đọc nửa câu đang hiện dần), nên phụ đề phải đứng yên
**1–2 lần `interval_ms`**; chớp nhanh hơn là mất trắng. Nhịp 1200 của bản Python đòi tới 2,4s —
quá lâu cho thoại game. Đo chi phí thật mỗi khung: **OneOCR 73ms, Windows OCR 16ms** (2 lượt),
nên ở nhịp 700ms chỉ tốn ~10% một nhân. Hạ xuống là gần như miễn phí.

Hai luật lọc dòng ngắn **chồng lấn nhau**, biết ranh giới thì đỡ chỉnh nhầm: `min_clean_ratio` đã dọn sạch
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

## Lấy lời thoại từ đâu

Chọn trong giao diện (**LẤY LỜI THOẠI TỪ**):

| Nguồn | Cần gì | Hợp với |
|---|---|---|
| **Màn hình** (mặc định) | 0 MB, chọn vùng phụ đề | game có phụ đề rõ |
| **Âm thanh** | tải 355 MB (chạy máy) hoặc API key (Speechmatics) | game không có phụ đề, hoặc phụ đề nằm ngoài vùng quét |

Nguồn âm thanh có hai bộ não:

| Nghe bằng | Cần gì | Đánh đổi |
|---|---|---|
| **Chạy trên máy** (mặc định) | tải 198 MB | miễn phí, không cần mạng, **không gửi gì ra ngoài** · **chỉ tiếng Anh** · ~5,5% một nhân |
| **Speechmatics** | API key | nghe chuẩn hơn khi nhạc nền to, nhiều thứ tiếng, chữ trả về có sẵn dấu câu · **lời thoại được gửi lên máy chủ của họ** |

Bản Speechmatics dùng WebSocket **đồng bộ** (`tungstenite`), không kéo tokio: khâu nghe vốn đã
chạy trong thread riêng nên thêm cả một runtime async chỉ để phục vụ một socket là thừa.

Nguồn âm thanh nghe **loopback** (mọi thứ đang phát ra loa) rồi nhận dạng tại chỗ bằng
sherpa-onnx streaming zipformer `en-2023-06-21` — offline, không key, không quota. Câu nghe được
đi vào đúng đường dịch + đọc của nguồn màn hình.

**Vì sao model chỉ-tiếng-Anh chứ không phải bản đa ngữ.** Đo trên hai file mẫu của sherpa, cùng
cấu hình, chỉ đổi model:

| | đa ngữ `multi-2025-02-10` | EN `en-2023-06-21` |
|---|---|---|
| `1.wav` | `God` … `Given her a lovely child` … `a blessed soul in he` | `God as a direct consequence of the sin, which man thus punished, had given her a love`/`Ly Child` … `a blessed soul in heaven.` |
| CPU | RTF 0,113 | **RTF 0,055** |
| Tải | 339 MB | **190 MB** |

Bản đa ngữ nuốt hẳn vế `as a direct consequence of the sin which man thus punished had` — nó đổi
độ chính xác tiếng Anh lấy khả năng đa ngôn ngữ. Bản EN train trên **LibriSpeech + GigaSpeech**;
GigaSpeech là podcast/YouTube, gần tiếng game hơn hẳn sách nói thu phòng thu. Muốn nghe tiếng
Trung/Nhật bằng nguồn âm thanh thì dùng Speechmatics, hoặc dùng nguồn màn hình.

### Ba khâu nối tiếp, thiếu một là hỏng

`nhận dạng → thêm dấu câu → gom → chẻ câu`. Bỏ khâu giữa thì bản chạy máy trả về **một mạch chữ
không có dấu chấm nào**, không chẻ được gì, cả đoạn 40 từ bị ném đi dịch làm một — dịch ra tối
nghĩa, TTS đọc liền chục giây, mà suốt chục giây đó cổng chặn đóng nên app **điếc hoàn toàn**.

Bộ gom cũng không bỏ được: bộ nhận dạng chốt theo khoảng lặng 0,8s còn người nói thì ngập ngừng,
nên thực tế bắt được `But` rồi `Me to get away from the information…` thành **hai mẩu rời**, và
`I` rồi `Think it do it…`. Dịch riêng từng mẩu thì mẩu đầu ra "Nhưng" cụt lủn, mẩu sau mất chủ
ngữ. Gom lại rồi mới cắt theo dấu câu thật là về đúng một câu.

**Giá CPU: RTF 0,055 — khoảng 5,5% một nhân.** Đây là con số phải đo trước khi làm, vì khác OCR
(chạy 16–73 ms mỗi 700 ms, có nghỉ), nhận dạng streaming phải nuốt 100% âm thanh liên tục.

### App tự nghe chính nó — và vì sao bịt tai là sai

ViNarrator phát tiếng Việt ra loa, mà loopback nghe cả tiếng đó. Không xử lý thì nó nhận dạng
giọng đọc của mình → dịch → đọc lại → lặp vô hạn.

Bản đầu chặn bằng cách **bịt tai suốt lúc đang đọc**. Chống được vòng lặp, nhưng đọc một câu
tiếng Việt mất xấp xỉ đúng thời gian câu tiếng Anh gốc, nên app câm mất khoảng một nửa:

| | Game nói | App làm gì | Tai |
|---|---|---|---|
| 0–3 s | câu **A** | nghe | mở |
| 3–3,5 s | câu **B** bắt đầu | dịch A | mở |
| 3,5–6,5 s | câu **B** tiếp | **đọc A** | **BỊT** |
| 6,8 s | câu **C** | nghe | mở |

Câu B mất trắng — mà không chỉ phần tiếng lúc bịt tai: lúc mở lại còn `reset` bộ nhận dạng nên
đoạn đầu B đã nghe được cũng bị vứt. Thực tế nghe được A, C, E, G.

Cách đúng nằm ở **tầng bắt tiếng**, không phải canh giờ. Windows 10 20H1 trở lên cho dựng
`IAudioClient` trên thiết bị ảo `VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK` với
`PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE` và PID của chính mình: dòng tiếng nhận được
chứa mọi thứ máy đang phát **trừ cây tiến trình ViNarrator**. Bịt tai thành thừa, nghe 100% thời
gian, và vòng lặp tự-nghe-mình trở thành bất khả thi về mặt cấu trúc.

`cpal` không mở được đường này nên phần đó gọi WASAPI trực tiếp (`src-tauri/src/loopback.rs`,
không thêm thư viện — crate `windows` vốn đã có sẵn cho OCR/TTS). Máy cũ hơn thì tự lùi về cpal
+ bịt tai như cũ và báo lên thanh trạng thái.

Test `loopback::tests::khong_nghe_thay_tieng_cua_chinh_minh` phát tiếng từ chính tiến trình đó
qua **cả hai đường phát của app** (`wave_out` cho VieNeu, `PlaySoundW` cho giọng Windows) rồi
soi biên độ bắt được: 0,0000. Đã chạy đỏ để chắc test phân biệt được — đổi sang
`INCLUDE_TARGET_PROCESS_TREE` thì đỉnh nhảy lên 0,634.

**Phải mồi im lặng trước khi nghe.** Zipformer khởi động với bộ đệm ngữ cảnh trái bằng 0 nên nuốt
mất đoạn đầu. Model EN hiện tại không còn cần, nhưng model đa ngữ trước đây thì cần thật:

| | Nghe ra |
|---|---|
| không mồi | `The yellow lamps would light up…` |
| mồi 1 giây | `After early nightfall the yellow lamps would light up…` |

Sự thật là `AFTER EARLY NIGHTFALL THE YELLOW LAMPS…`, tức **mất trắng 2 giây đầu**. Đã loại trừ
các nghi phạm khác trước khi kết luận: nạp nguyên khối một lần và tắt hẳn endpoint đều ra y hệt.

Hai chỗ khác cũng phải chỉnh so với bản gốc: **phải xả** khi dừng (sherpa chỉ nhả câu ở mốc im
lặng, tiếng dứt đột ngột thì chữ kẹt lại — file mẫu 6,6s ra **rỗng hoàn toàn**), và **nới
`rule3` 6s → 12s** (nó cắt theo đồng hồ chứ không theo chỗ nghỉ nên rơi vào giữa từ:
`…her a love` / `Ly child whose place…`, đem dịch thì ra rác).

Cái giá của tính năng này: exe 13,6 → **31,6 MB**, installer 3,4 → **8,0 MB** (sherpa-onnx link
tĩnh). Không cần cmake/libclang — thư viện native tải sẵn dạng prebuilt lúc build.

## Giọng đọc

Chọn trong giao diện (**GIỌNG ĐỌC**):

| | tần số | tiếng đầu¹ | đĩa | RAM | ghi chú |
|---|---|---|---|---|---|
| **Giọng Windows** (mặc định) | 16 kHz | 0,26s | 0 MB | — | có sẵn; cũng là **giọng dự phòng** khi VieNeu trục trặc |
| **VieNeu-TTS v3 Turbo** | 48 kHz | **0,18s** | 561 MB | 1,3 GB | 14 giọng, **đọc lẫn Anh–Việt**, phát theo mẩu |

¹ thời gian tới lúc **nghe thấy tiếng**, không phải tới lúc xong cả câu. Cùng máy, câu 144 ký tự.

Bốn model đã tải về nghe thử trước khi chọn:

| | RTF | kết quả |
|---|---|---|
| **VieNeu-TTS v3 Turbo** | 0,41 | 48 kHz, ~10.000 giờ Anh–Việt — **chọn** |
| Piper `vi_VN-vais1000` | 0,05 | nhẹ nhất (82 MB, 97 MB RAM) nhưng **nghe thử thì đọc chữ tiếng Anh không ổn** — đã bỏ, xem dưới |
| MMS-TTS-vie (Meta) | 0,22 | chậm mà không hơn rõ rệt |
| V-TTS / Valtec | — | trọng số `v-tts/v-tts-pretrained` trả **401**; bản `.exe` 394 MB thì **GUI thuần** (không gọi được từ code) và **CC BY-NC 4.0** |

**Vì sao bỏ Piper.** Bộ phiên âm espeak-ng cho đúng âm tiếng Anh (`bˈɒs`, `ʃˈɪft`,
`ɡˈeɪm ˌəʊvə` — kiểm bằng `piper --debug`), nhưng giọng `vais1000` chỉ được huấn luyện trên
tiếng Việt nên **không phát ra nổi mấy âm đó**. Phụ đề game đầy tên riêng và thuật ngữ tiếng
Anh, nghe thử thì không dùng được. Giữ hai engine neural song song chỉ để phòng hờ là không
đáng, vì giọng Windows đã đủ làm lưới an toàn.

**Bài học đo được:** ban đầu VieNeu bị loại vì "3,7s/câu". Sai — 3,7s là cho **cả câu 9 giây**.
Thư viện có sẵn `infer_stream` (**model card không ghi**) cho mẩu tiếng đầu sau 0,18s, và sinh
nhanh gấp ~2,3 lần tốc độ phát nên không bao giờ hụt. Cái chậm là **kiến trúc app** chờ WAV
hoàn chỉnh, không phải model.

### Ảnh hưởng tới game — đo lúc CPU bị tranh

App **luôn chạy nền trong lúc người dùng đang chơi game**, nên chỉ đo giây/câu là thiếu.
Đo với 5 tiến trình 1 luồng giả làm game + 1 luồng khung hình (vừa kín 6 nhân vật lý
i5-12400F), tiến trình con ở BELOW_NORMAL, so với nền:

| | p50 | p99 | max (giật) | RAM |
|---|---|---|---|---|
| **VieNeu** (`infer_stream`, `threads=2`) | +17,7% | **+6,2%** | **+0,1%** | 1,3 GB |
| *(Piper — đã bỏ, để đối chiếu)* | +7,8% | +7,9% | +36,2% | 97 MB |

VieNeu chạy **đều 1,5 nhân** nên gần như không gây giật; Piper rẻ hơn về trung bình nhưng bùng
5,8 nhân trong 0,39s nên vẫn còn cú giật sót. Phép đo là **trường hợp xấu nhất**: "game" là
vòng nhân ma trận tổng hợp (không có đồng bộ GPU), và TTS bị bơm liên tục, dày hơn phụ đề thật
nhiều lần.

**`threads=2` là con số đo được**, không phải đoán: so với mặc định (6 luồng) nó vừa chiếm ít
nhân hơn (1,50 so với 1,93) vừa **tốn ít CPU hơn** (5,89 so với 8,55 CPU-giây/câu, bớt đồng bộ
luồng) mà độ trễ gần như không đổi.

#### Ưu tiên tiến trình: bắt buộc, không phải tinh chỉnh cho vui

Đo trên Piper (onnxruntime bung ~5,8 nhân trong 0,39s — đúng dạng gây khựng khung hình):

| ưu tiên tiến trình con | p50 | p99 | max (giật) |
|---|---|---|---|
| thường | +5,0% | +126,6% | **+183,7%** |
| **BELOW_NORMAL** (đang dùng) | +7,8% | **+7,9%** | **+36,2%** |
| IDLE | +7,7% | +12,0% | +36,8% |

Một cờ `creation_flags` đổi p99 từ **+127% xuống +8%**. IDLE quá tay, không hơn.

**Ghim nhân KHÔNG thay được cách này** — đã thử và thất bại, có kiểm chứng:

- `SetProcessAffinityMask` đặt xong bị **chính onnxruntime ghi đè**: đặt `[0]`, chạy một câu,
  đọc lại ra `[0..11]`. Bảng đo affinity đầu tiên vì thế đo vào hư không.
- `OMP_NUM_THREADS`, `ORT_INTRA_OP_NUM_THREADS`: **không tác dụng** (5,85 / 5,86 / 5,64 nhân).

### VieNeu chạy bằng gì

VieNeu là thư viện **Python**, nên app tự nuôi một môi trường Python riêng — **không** dùng
Python có sẵn của máy (đã thấy `uv` vớ nhầm Python 3.12 của người dùng; app phải xác định
trước môi trường của mình).

```
%LOCALAPPDATA%\ViNarrator\vieneu\
  uv.exe    46 MB   một file tĩnh, tải từ astral-sh/uv (ghim 0.12.1)
  py\       60 MB   uv tự tải CPython 3.12 (--python-preference only-managed)
  venv\    163 MB   uv pip install --no-deps vieneu + 8 gói thật sự cần
  hf\      285 MB   model, kéo về lúc chạy nóng
```

Cài `--no-deps` rồi tự thêm từng gói là **cố ý**: bản đầy đủ kéo theo `gradio`/`librosa`/
`perth`/`pandas`/`sklearn` thành **713 MB**, bản rút gọn **163 MB** mà tổng hợp y hệt (đã kiểm).

Đo được: **65 giây, 561 MB** từ trạng thái trắng trên mạng nhanh.

Ba chi tiết:

- **Phát theo mẩu, qua hàng đợi `waveOut`** (`wave_out.rs`) chứ **không** dùng `PlaySound`.
  Đây là bug thật đã gặp: `PlaySound` mở lại thiết bị âm thanh ở mỗi lần gọi, đo được
  **+71,6 ms mỗi ranh giới mẩu** (lớn nhất +110 ms, tổng **+0,646s** chèn vào câu 9,44s).
  Và `infer_stream` cắt theo *frame* chứ không theo cụm từ — tôi đã đoán sai chỗ này — nên
  khe rơi vào **giữa từ**, nghe giật rất rõ. Cùng đống mẩu ấy ghép sẵn thành một file rồi phát
  một lần chỉ lệch +0,112s. Chuyển sang `waveOut` xếp hàng nhiều đệm: khe còn **+0,017s**
  (~2 ms mỗi ranh giới), giảm 35 lần. Test `phat_lien_mach_khong_co_khe` giữ chỗ này — nó
  **trừ đi độ trễ mẩu đầu** để đo đúng khe chứ không đo chi phí cố hữu của streaming.
- **Bên Rust tự đếm số thứ tự mẩu**, không đọc tên file worker in ra:
  đường dẫn có thể chứa tiếng Việt.
- **Bấm DỪNG giữa câu** vẫn phải rút hết tin của câu đó khỏi kênh, nếu không câu sau sẽ đọc
  nhầm mẩu thừa của câu trước. Có test `dung_giua_cau_thi_im_ngay` giữ chỗ này.
- **`worker.py` được ghi lại ở MỌI lần dựng worker**, không chỉ lúc cài. `ensure_runtime`
  thoát sớm khi đã cài xong, nên nếu chỉ ghi ở đó thì **cập nhật app vẫn chạy worker của bản
  cũ** — đã dính đúng lỗi này: sửa worker để nó báo danh sách giọng, chạy lại, worker cũ vẫn
  chạy và không có giọng nào. Ghi vài KB mỗi lần dựng rẻ hơn nhiều so với đi gỡ lỗi đó.
- **Danh sách giọng do chính model báo** (`V <tên>` trước `READY`, lưu ra `voices.txt`), UI
  không chép cứng. Test đòi ít nhất 5 giọng nên worker cũ/lệch giao thức sẽ bị bắt ngay.

Ba thứ chống "hỏng mà không biết vì sao":

- **stderr của cả hai worker được đọc ra log**, dòng cuối ghép vào thông báo lỗi. Trước đây
  `Stdio::null()` nuốt sạch traceback Python — hỏng gì cũng chỉ thấy "worker không trả lời".
- **Hai mức chờ READY**: 900s lúc cài (còn phải tải model), **60s lúc chạy**. Dùng chung mức
  900s thì worker kẹt sẽ treo luồng đọc 15 phút mà không ai biết.
- **Kiểm đĩa trống trước khi tải** (cần 1,5 GB). Hết đĩa giữa chừng để lại
  đống dở, `ready()` false, lần sau cài lại từ đầu mà người dùng không hiểu vì sao.

Engine được **hâm nóng lúc rảnh** ngay khi chọn hoặc vừa cài xong, không đợi câu đầu tiên:
nạp model tốn ~10s, để đến lúc có câu mới dựng thì vừa câm 10s vừa mất 2–3 câu phụ đề trôi
qua (hàng đợi chỉ giữ 3).

## Kiến trúc

| File | Vai trò | Port từ (bản Python) |
|------|---------|------------------------|
| `src-tauri/src/capture.rs` | chụp vùng màn hình BGRA + tô trắng vùng loại trừ | `capture.py`, `mask_exclusion` |
| `src-tauri/src/ocr.rs` | chọn engine + `Windows.Media.Ocr` | `ocr.py` (winsdk) |
| `src-tauri/src/oneocr.rs` | OneOCR (Snipping Tool) + tự chép runtime | — (mới) |
| `src-tauri/src/translate.rs` | Google Translate + LLM (OpenAI-compatible) | `translate.py` (deep-translator) |
| `src-tauri/src/tts.rs` | chọn giọng + `Windows.Media.SpeechSynthesis` + `PlaySound` | `tts.py` (winsdk + winsound) |
| `src-tauri/src/asr.rs` | nghe loopback, cổng chặn tiếng của chính app, gom + chẻ câu | — (mới) |
| `src-tauri/src/speechmatics.rs` | nhận dạng qua Speechmatics (WebSocket đồng bộ) | — (mới) |
| `src-tauri/src/stretch.rs` | đổi tốc độ đọc mà giữ cao độ (WSOLA) | — (mới) |
| `src-tauri/src/runtime.rs` | tải file + tiến độ, đọc stderr worker, kiểm đĩa, chỉnh âm lượng | — (mới) |
| `src-tauri/src/vieneu.rs` | giọng VieNeu + tự nuôi Python bằng `uv` | — (mới) |
| `src-tauri/src/vieneu_worker.py` | worker Python, nhúng vào exe rồi ghi ra đĩa lúc cài | — (mới) |
| `src-tauri/src/wave_out.rs` | phát liền mạch bằng hàng đợi đệm `waveOut` (cho VieNeu) | — (mới) |
| `src-tauri/src/dedup.rs` | chống đọc trùng phụ đề cuộn | `dedup.py` |¹
| `src-tauri/src/speech_queue.rs` | hàng đợi đọc (dedup + cap 3) | `speech_queue.py` |
| `src-tauri/src/narrator.rs` | chụp→OCR→dedup **hoặc** nghe→nhận dạng, rồi dịch→đọc | `narrator.py` |
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

Thread này **phải tự kiểm tra cờ `running`**. `stop_playback()` chỉ xoá hàng đợi TTS; nếu thread
dịch cứ dịch nốt rồi `speak()` thì nạp lại vào đúng cái hàng đợi vừa xoá — bấm DỪNG xong vẫn
nghe thêm mấy câu. Đo bằng server dịch giả (cố tình chậm 1,5s/câu): bản chưa vá gửi request
**sau khi bấm dừng 1 giây**; bản đã vá dừng hẳn, theo dõi thêm 8s không có gì.

Unit test của bản Python (`tests/test_dedup.py`, `tests/test_speech_queue.py`) đã port nguyên
sang Rust và đều pass — dùng để kiểm chứng bản port. Cột "port từ" trỏ tới file của bản Python,
xem được ở commit `64e09eb`.

## Bản Python cũ

Đã xoá khỏi cây làm việc. Cần đối chiếu thì lấy từ git:

```bash
git show 64e09eb:gui.py          # xem 1 file
git checkout 64e09eb -- narrator.py ocr.py tts.py   # lấy lại vài file
```

Còn một thứ của bản Python **chưa có** ở bản Rust:

| Bỏ | Vì sao | Muốn có lại |
|---|---|---|
| Chế độ "Đọc thuần" (OCR tiếng Việt, EasyOCR) | Windows OCR không hỗ trợ tiếng Việt | nhúng PaddleOCR ONNX qua crate `ort` |

*(Giọng neural đã có lại bằng VieNeu — xem **Giọng đọc** ở trên. Piper và MMS bỏ hẳn.)*

Thư mục `models/` (~200 MB) cũng đã xoá và nằm trong `.gitignore` — `model.safetensors` 145 MB
vượt giới hạn 100 MB của GitHub. Model của VieNeu do app tự tải về `%LOCALAPPDATA%`, không cần
đụng tay.

Muốn nghe lại mấy model đã loại để tự kiểm chứng:

- Piper `vi_VN-vais1000-medium.onnx` (+ `.onnx.json`) —
  [rhasspy/piper-voices](https://huggingface.co/rhasspy/piper-voices/tree/main/vi/vi_VN)
- MMS `mms-tts-vie` — [Xenova/mms-tts-vie](https://huggingface.co/Xenova/mms-tts-vie)
