//! Nguồn **âm thanh**: nghe tiếng game ở loopback rồi nhận dạng tại chỗ (sherpa-onnx streaming
//! zipformer). Dành cho game KHÔNG có phụ đề, hoặc phụ đề nằm ngoài vùng quét.
//!
//! Chạy hoàn toàn offline, không key, không quota. Câu nhận được đi vào đúng đường dịch + đọc
//! của nguồn màn hình — xem [`crate::narrator`].
//!
//! Hai bộ não để chọn: **chạy máy** (zipformer EN int8, 190 MB + 7,6 MB dấu câu — chỉ tiếng
//! Anh, không cần mạng, ~5,5% một nhân) và **Speechmatics** (đám mây, cần API key, nhiều thứ
//! tiếng, chữ trả về đã có sẵn dấu câu). Phần bắt loopback, cổng chặn và gom câu dùng chung —
//! xem [`Nao`]. Muốn nghe tiếng Trung/Nhật bằng nguồn âm thanh thì phải dùng Speechmatics; bản
//! chạy máy cố tình chọn model chuyên tiếng Anh vì bản đa ngữ nghe tiếng Anh kém hơn hẳn (xem
//! [`REPO`]).
//!
//! **Vì sao không có khử vọng (AEC).** Bản gốc mà kiến trúc này bê về (RiTaiAI) bắt HAI luồng —
//! mic và loopback — nên phải khử tiếng loa lọt vào mic. Ở đây không có mic, chỉ loopback, nên
//! bỏ được cả `sonora` lẫn khâu đồng bộ khung 10ms.
//!
//! **Cái phải xử lý thay vào đó: app tự nghe chính nó.** ViNarrator phát tiếng Việt ra loa, mà
//! loopback nghe tất cả những gì loa phát — kể cả tiếng của chính nó. Không chặn thì nó nhận
//! dạng giọng đọc của mình, dịch, rồi đọc lại, lặp vô hạn.
//!
//! Đường chính giải quyết chuyện đó ở **tầng bắt tiếng**: [`crate::loopback`] xin Windows một
//! dòng tiếng đã loại sẵn cây tiến trình của app, nên không có gì để chặn. Cổng [`Cong`] chỉ
//! còn dùng cho máy quá cũ phải lùi về cpal.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SupportedStreamConfig};
use sherpa_onnx::{
    OnlinePunctuation, OnlinePunctuationConfig, OnlinePunctuationModelConfig, OnlineRecognizer,
    OnlineRecognizerConfig, OnlineStream,
};

use crate::runtime::{self, NO_WINDOW};
use crate::{loopback, nhat_ky, speechmatics};

/// Model **chỉ tiếng Anh** `streaming-zipformer-en-2023-06-21` (encoder+joiner int8, decoder fp32).
///
/// **Đo trên chính hai file mẫu của sherpa, cùng cấu hình, chỉ đổi model:**
///
/// | | đa ngữ `multi-2025-02-10` | EN `en-2023-06-21` |
/// |---|---|---|
/// | `1.wav` | `God` … `Given her a lovely child` … `a blessed soul in he` | `God as a direct consequence of the sin, which man thus punished, had given her a love`/`Ly Child` … `a blessed soul in heaven.` |
/// | CPU | RTF 0,113 = 11,3% một nhân | **RTF 0,055 = 5,5%** |
/// | Tải | 339 MB | **190 MB** |
///
/// Tức bản đa ngữ **nuốt hẳn vế** `as a direct consequence of the sin which man thus punished
/// had` và cụt `in heaven` — tôi từng kết luận "model nghe không ra đoạn đó"; đúng hơn phải là
/// *model đa ngữ* nghe không ra. Nó đổi độ chính xác tiếng Anh lấy khả năng đa ngôn ngữ, mà app
/// này chỉ cần tiếng Anh (chế độ Trung→Việt vẫn còn nguồn màn hình).
///
/// Bản này train trên **LibriSpeech + GigaSpeech**. GigaSpeech là podcast/YouTube — thoại thật,
/// có nhạc nền, giọng đời thường; gần tiếng game hơn hẳn LibriSpeech (sách nói thu phòng thu).
/// Bản EN 20M từng thử và bị chê nghe không chuẩn là LibriSpeech-only *và* nhỏ nhất họ có.
///
/// Đổi luôn nguồn tải: **HuggingFace của csukuangfj**, chính người phát hành sherpa-onnx, thay
/// cho bản re-host trên release cá nhân `din730199/ritai-releases` mà bản trước phải mượn.
/// Không lấy tarball trên GitHub release vì nó gói kèm cả bản fp32 — 507 MB tải để giữ 190.
const REPO: &str =
    "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-2023-06-21/resolve/main";
const ENC: &str = "encoder-epoch-99-avg-1.int8.onnx";
const DEC: &str = "decoder-epoch-99-avg-1.onnx";
const JOIN: &str = "joiner-epoch-99-avg-1.int8.onnx";
const TOKENS: &str = "tokens.txt";

/// Model DẤU CÂU (`sherpa-onnx-online-punct-en-2024-08-06`, bản int8). Bắt buộc phải có, không
/// phải trang trí: bộ nhận dạng trả về một mạch chữ **không có dấu chấm nào**, mà không có dấu
/// chấm thì [`chia_cau`] không cắt được gì và cả đoạn 40 từ bị ném đi dịch làm một — dịch ra
/// tối nghĩa, rồi TTS đọc liền chục giây, và suốt chục giây đó cổng đóng nên **điếc hoàn toàn**.
///
/// Chỉ có ở dạng tarball trên GitHub release (HuggingFace không có bản lẻ — đã thử, trả 401).
/// Tải 37 MB nhưng chỉ giữ lại 7,6 MB: bản fp32 trong đó app không đụng tới.
const PUNCT_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/punctuation-models/sherpa-onnx-online-punct-en-2024-08-06.tar.bz2";
const PUNCT_THU_MUC: &str = "sherpa-onnx-online-punct-en-2024-08-06";
const PUNCT_MODEL: &str = "model.int8.onnx";
const PUNCT_VOCAB: &str = "bpe.vocab";

/// Kèm kích thước THẬT (lấy từ HuggingFace API rồi đối chiếu lại bằng `HEAD` trên đúng URL tải).
/// Dùng để bắt file tải dở: `runtime::fetch` ghi `.part` rồi mới đổi tên, nhưng nếu server cắt
/// giữa chừng mà không báo lỗi thì vẫn ra một file ngắn mang tên đúng.
const FILES: [(&str, u64); 4] =
    [(ENC, 187_823_992), (DEC, 2_092_566), (JOIN, 259_335), (TOKENS, 5_048)];
const PUNCT_FILES: [(&str, u64); 2] = [(PUNCT_MODEL, 7_490_500), (PUNCT_VOCAB, 149_430)];

/// Đuôi câu chưa có dấu kết mà im lặng quá ngần này thì chốt luôn, đừng giữ mãi trong bộ gom.
/// Nghe tiếng người nói ngập ngừng ("But…", "I…") sinh ra rất nhiều mẩu một từ; giữ lâu quá thì
/// bản dịch tới muộn, chốt sớm quá thì lại chẻ vụn đúng cái tật đang phải sửa.
///
/// 1,2s như RiTaiAI (`AGG_SILENCE_MS`). Trước để 1,5s cho chắc, nhưng giờ chữ tạm đã lùi được
/// hạn khi người ta còn đang nói ([`Gom::con_noi`]) nên không cần phần dôi ra đó nữa — cắt 300ms
/// khỏi độ trễ của mọi câu bỏ lửng.
const IM_CHOT: Duration = Duration::from_millis(1200);

/// Nạp sherpa theo từng mẩu 100ms. Nhỏ hơn thì tốn phí gọi hàm, lớn hơn thì trễ chữ.
///
/// **Tính theo tần số chứ không phải số mẫu cố định.** Trước để cứng 3200 mẫu, hợp với 48 kHz
/// của cpal (~67ms); nhưng đường bắt tiếng mới xin thẳng 16 kHz, cùng con số đó thành 200ms —
/// gấp ba, và mốc ngắt câu cũng thô đi đúng chừng ấy.
fn chunk(sr: u32) -> usize {
    sr as usize / 10
}

/// Mồi bộ nhận dạng bằng ngần này im lặng trước khi cho nghe tiếng thật — xem [`mo_stream`].
const MOI_GIAY: f32 = 1.0;

/// Giữ chặn thêm ngần này sau khi TTS báo đọc xong. Thiết bị âm thanh còn đang xả đệm khi
/// `WaveOut::drain` đã trả về, không có khoảng nhả này thì cái đuôi lọt vào và bị nhận dạng.
const NHA_CONG: Duration = Duration::from_millis(300);

pub fn dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ViNarrator")
        .join("asr")
}

/// Đủ 4 file và đúng kích thước. Không dùng file marker riêng như VieNeu vì ở đây chỉ có tải
/// file, không có bước cài đặt nào có thể hỏng nửa chừng mà file vẫn đủ.
pub fn ready() -> bool {
    let d = dir();
    FILES
        .iter()
        .chain(PUNCT_FILES.iter())
        .all(|(f, n)| std::fs::metadata(d.join(f)).map(|m| m.len() == *n).unwrap_or(false))
}

/// Tải model (~198 MB, gồm 7,6 MB dấu câu). Gọi lại khi đã đủ thì trả về ngay.
pub fn ensure_model(bao: runtime::BaoTien) -> Result<(), String> {
    if ready() {
        return Ok(());
    }
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    don_rac(&d);
    // 198 MB file giữ lại + 37 MB tarball dấu câu nằm tạm cùng chỗ trước khi bóc.
    runtime::du_dia(&d, 300)?;
    for (f, n) in FILES {
        let dest = d.join(f);
        if du(&dest, n) {
            continue;
        }
        runtime::fetch(&format!("{REPO}/{f}"), &dest, f, bao)?;
        kiem(&dest, n, f)?;
    }
    if !PUNCT_FILES.iter().all(|(f, n)| du(&d.join(f), *n)) {
        bao("Đang tải model dấu câu…".into());
        let tar = d.join("punct.tar.bz2");
        runtime::fetch(PUNCT_URL, &tar, "dấu câu", bao)?;
        // Bóc ĐÚNG hai file cần, bỏ lại bản fp32 29,6 MB nằm chung trong tarball.
        //
        // KHÔNG dùng `--strip-components`: `Command::new("tar")` lấy `tar` đầu tiên trên PATH,
        // mà máy có Git thì đó là `tar` của Git — bản đó nuốt cờ này rồi vẫn tạo thư mục con
        // (đã dính: file không xuất hiện, chỉ có thư mục rỗng). `tar.exe` của Windows thì làm
        // đúng. Không đoán được máy người dùng có cái nào, nên tự dời file cho chắc.
        let st = Command::new("tar")
            .arg("-xf")
            .arg(&tar)
            .arg("-C")
            .arg(&d)
            .arg(format!("{PUNCT_THU_MUC}/{PUNCT_MODEL}"))
            .arg(format!("{PUNCT_THU_MUC}/{PUNCT_VOCAB}"))
            .creation_flags(NO_WINDOW)
            .status()
            .map_err(|e| format!("chạy tar: {e}"))?;
        let _ = std::fs::remove_file(&tar);
        if !st.success() {
            return Err(format!("giải nén model dấu câu lỗi (tar {st})"));
        }
        let con = d.join(PUNCT_THU_MUC);
        for (f, _) in PUNCT_FILES {
            // Bản tar nào strip sẵn thì file đã nằm đúng chỗ -> không có gì để dời, bỏ qua lỗi.
            let _ = std::fs::rename(con.join(f), d.join(f));
        }
        let _ = std::fs::remove_dir_all(&con);
        for (f, n) in PUNCT_FILES {
            kiem(&d.join(f), n, f)?;
        }
    }
    Ok(())
}

/// Xoá mọi file lạ trong thư mục model.
///
/// **Vì đổi model là đổi luôn tên file.** Người đã cài bản đa ngữ có 339 MB mang tên
/// `encoder-epoch-75-avg-11-…` nằm lại vĩnh viễn: [`ready`] chỉ soi [`FILES`] nên không ai
/// nhắc, mà tên khác nhau nên tải bản mới cũng không đè lên. Chỉ chạy trên đường CÀI (đã qua
/// `ready()` ở đầu [`ensure_model`]) nên lần chạy bình thường không đụng tới ổ đĩa.
///
/// Thư mục này do app tạo và chỉ chứa model, không có gì của người dùng để mất.
fn don_rac(d: &Path) {
    let Ok(muc) = std::fs::read_dir(d) else {
        return;
    };
    for f in muc.flatten() {
        let ten = f.file_name();
        let giu = FILES.iter().chain(PUNCT_FILES.iter()).any(|(g, _)| ten == *g);
        if !giu && f.path().is_file() {
            let _ = std::fs::remove_file(f.path());
        }
    }
}

fn du(p: &Path, n: u64) -> bool {
    std::fs::metadata(p).map(|m| m.len() == n).unwrap_or(false)
}

fn kiem(p: &Path, n: u64, ten: &str) -> Result<(), String> {
    let co = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    if co == n {
        return Ok(());
    }
    let _ = std::fs::remove_file(p);
    Err(format!("{ten} tải về sai kích thước: {co}/{n} byte"))
}

/// Nạp model dấu câu. Thiếu file thì chạy tiếp KHÔNG có dấu câu thay vì chết — nhưng lúc đó
/// [`chia_cau`] mất tác dụng nên câu sẽ dài lê thê, xem [`PUNCT_URL`].
fn nap_dau_cau(d: &Path) -> Option<OnlinePunctuation> {
    let c = OnlinePunctuationConfig {
        model: OnlinePunctuationModelConfig {
            cnn_bilstm: Some(d.join(PUNCT_MODEL).to_string_lossy().to_string()),
            bpe_vocab: Some(d.join(PUNCT_VOCAB).to_string_lossy().to_string()),
            num_threads: 1,
            provider: Some("cpu".into()),
            debug: false,
        },
    };
    OnlinePunctuation::create(&c)
}

pub fn setup(d: &Path) -> Result<OnlineRecognizer, String> {
    let p = |f: &str| d.join(f).to_string_lossy().to_string();
    let mut c = OnlineRecognizerConfig::default();
    c.model_config.transducer.encoder = Some(p(ENC));
    c.model_config.transducer.decoder = Some(p(DEC));
    c.model_config.transducer.joiner = Some(p(JOIN));
    c.model_config.tokens = Some(p(TOKENS));
    c.model_config.provider = Some("cpu".into());
    // greedy_search chứ không modified_beam_search: đã thử, beam search vừa CHẬM hơn (RTF
    // 0,129 so với 0,116) vừa ra rác — cùng file mẫu nó đẻ thêm hai mẩu "H" và "Conf" ở đầu.
    c.decoding_method = Some("greedy_search".into());
    // Ngắt câu. Đường CHÍNH là rule2: im lặng 0,8s sau khi đã nói -> chốt. Thoại game gần như
    // lúc nào cũng có nghỉ giữa các câu nên đường này gánh hầu hết.
    //
    // rule3 là lưới an toàn cho lúc nói một hơi không nghỉ. Tôi từng nới lên 12s để tránh nó
    // cắt giữa từ, nhưng như thế một đoạn độc thoại phải chờ tới 12 giây mới nghe được bản
    // dịch — chậm rõ rệt khi chơi. Về đúng 6s như RiTaiAI: thà thỉnh thoảng lệch một từ ở chỗ
    // nối còn hơn ngồi đợi.
    c.enable_endpoint = true;
    // rule1 (im lặng mà CHƯA ra chữ nào) TẮT hẳn, khác RiTaiAI để 1,6s: app này không cần chốt
    // lúc chưa ai nói, nên rule1 chẳng có việc gì làm ngoài việc đẻ ra mốc ngắt rỗng rồi reset
    // theo. Với model đa ngữ trước đây nó còn tai hại thật — sau mỗi reset model đó câm 2 giây
    // để dựng lại 128 khung ngữ cảnh trái, rule1 tưởng là im lặng nên reset tiếp, càng reset
    // càng câm (đo được: bật thì ra `1.80s "GOD" · 3.80s "" · 9.80s "GIVEN HER…"`, mất một vế).
    c.rule1_min_trailing_silence = 100.0;
    c.rule2_min_trailing_silence = 0.8;
    c.rule3_min_utterance_length = 6.0;
    OnlineRecognizer::create(&c)
        .ok_or_else(|| "không dựng được bộ nhận dạng — model hỏng hoặc tải dở".into())
}

/// Dựng stream rồi **mồi bằng một quãng im lặng** trước khi cho nghe tiếng thật.
///
/// **Model EN đang dùng KHÔNG cần cái này nữa** — đo lại trên `0.wav`, mọi thứ khác giữ nguyên:
///
/// | | Nghe ra |
/// |---|---|
/// | không mồi | `After early nightfall, the yellow lamps…` |
/// | mồi 1s | `After early nightfall, the yellow lamps…` |
///
/// Giữ lại vì nó chỉ tốn một giây im lặng **đúng một lần lúc bật**, còn model cũ thì cần thật:
/// bản đa ngữ `chunk-16-left-128` khởi động với 128 khung ngữ cảnh trái bằng 0 nên nuốt trắng
/// 2 giây đầu (`The yellow lamps would light up…` — mất hẳn `After early nightfall`). Đổi model
/// lần nữa mà bỏ mất chỗ này thì lỗi đó quay lại im lặng, nên để nguyên cho rẻ.
///
/// Hồi đó đã loại trừ hết nghi phạm khác trước khi kết luận: nạp nguyên khối một lần (đúng ví
/// dụ của crate) và tắt hẳn `enable_endpoint` đều cho kết quả y hệt lúc chưa mồi.
fn moi(rec: &OnlineRecognizer, st: &OnlineStream, sr: u32) {
    st.accept_waveform(sr as i32, &vec![0.0f32; (sr as f32 * MOI_GIAY) as usize]);
    while rec.is_ready(st) {
        rec.decode(st);
    }
    rec.reset(st);
}

fn mo_stream(rec: &OnlineRecognizer, sr: u32) -> OnlineStream {
    let st = rec.create_stream();
    moi(rec, &st, sr);
    st
}

fn ket_cau(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…')
}

fn dong_nhay(c: char) -> bool {
    matches!(c, '"' | '\'' | '”' | '’' | ')' | ']' | '»')
}

/// Cắt ra các câu HOÀN CHỈNH, trả lại phần đuôi còn dở.
///
/// Chỉ nhận điểm cắt khi sau chuỗi dấu kết (+ dấu đóng nháy nếu có) là khoảng trắng hoặc hết
/// chuỗi — nhờ vậy `3.14` không bị xẻ đôi. Không phân biệt được `Mr.`, chấp nhận.
fn chia_cau(s: &str) -> (Vec<String>, String) {
    let mut cau = Vec::new();
    let mut con = s;
    loop {
        let ky: Vec<(usize, char)> = con.char_indices().collect();
        let mut het: Option<usize> = None;
        let mut i = 0;
        while i < ky.len() {
            if !ket_cau(ky[i].1) {
                i += 1;
                continue;
            }
            let mut j = i;
            while j < ky.len() && ket_cau(ky[j].1) {
                j += 1;
            }
            while j < ky.len() && dong_nhay(ky[j].1) {
                j += 1;
            }
            if j >= ky.len() {
                het = Some(con.len());
                break;
            }
            if ky[j].1.is_whitespace() {
                het = Some(ky[j].0);
                break;
            }
            i += 1; // không phải ranh giới thật (vd "3.14") -> tìm dấu kết tiếp theo
        }
        match het {
            Some(e) => {
                let c = con[..e].trim();
                if c.is_empty() {
                    break;
                }
                cau.push(c.to_string());
                con = &con[e..];
            }
            None => break,
        }
    }
    (cau, con.trim().to_string())
}

fn noi(a: &str, b: &str) -> String {
    match (a.trim(), b.trim()) {
        ("", y) => y.to_string(),
        (x, "") => x.to_string(),
        (x, y) => format!("{x} {y}"),
    }
}

/// Gom các mẩu vụn thành câu trọn vẹn trước khi đem đi dịch.
///
/// **Vì sao cần.** Bộ nhận dạng chốt câu theo khoảng lặng 0,8s, mà người nói thì ngập ngừng —
/// thực tế bắt được `But` rồi `Me to get away from the information…` là hai mẩu rời, và `I` rồi
/// `Think it do it…`. Đem từng mẩu đi dịch thì `But` ra "Nhưng" cụt lủn còn mẩu sau mất chủ ngữ.
/// Gom lại rồi mới cắt theo dấu câu thật thì cả hai về đúng một câu.
///
/// Đuôi chưa có dấu kết mà im quá [`IM_CHOT`] thì chốt luôn — người nói bỏ lửng câu là chuyện
/// thường, giữ mãi thì bản dịch không bao giờ tới.
#[derive(Default)]
struct Gom {
    dem: String,
    han: Option<Instant>,
}

impl Gom {
    fn them(&mut self, t: &str, gio: Instant, ra: &dyn Fn(String)) {
        self.dem = noi(&self.dem, t);
        let (cau, con) = chia_cau(&self.dem);
        for c in cau {
            ra(c);
        }
        self.dem = con;
        self.han = (!self.dem.is_empty()).then_some(gio + IM_CHOT);
    }

    /// Người ta vẫn đang nói (Speechmatics còn bắn chữ tạm về) — lùi hạn chốt đuôi.
    ///
    /// Không có cái này thì một câu nói ngập ngừng bị chẻ đôi: máy chủ chỉ gửi bản chốt khi có
    /// từ mới, ngập ngừng quá [`IM_CHOT`] là bộ gom hết giờ và ném nửa câu đi dịch.
    fn con_noi(&mut self, gio: Instant) {
        if self.han.is_some() {
            self.han = Some(gio + IM_CHOT);
        }
    }

    fn nhip(&mut self, gio: Instant, ra: &dyn Fn(String)) {
        if self.han.is_some_and(|h| gio >= h) {
            self.xa(ra);
        }
    }

    fn xa(&mut self, ra: &dyn Fn(String)) {
        let t = std::mem::take(&mut self.dem);
        self.han = None;
        if t.chars().any(char::is_alphanumeric) {
            ra(t);
        }
    }
}

/// Cổng chặn tiếng của CHÍNH APP.
///
/// ViNarrator phát tiếng Việt ra loa; loopback nghe lại chính nó. Không có cổng này thì:
/// nghe giọng đọc của mình → nhận dạng → dịch → đọc → nghe lại → lặp vô hạn.
///
/// Đóng cổng lúc TTS đang đọc, và giữ đóng thêm [`NHA_CONG`] sau đó cho thiết bị xả hết đệm.
/// Lúc mở lại thì `reset` bộ nhận dạng: nửa câu dở dang trước khi đóng cổng không còn nối được
/// với tiếng mới, để nguyên sẽ ghép thành câu vô nghĩa.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Cua {
    /// TTS đang đọc (hoặc còn trong khoảng nhả) — vứt tiếng đi.
    Dong,
    /// Nhịp đầu tiên sau khi hết chặn — phải reset bộ nhận dạng.
    VuaMo,
    Mo,
}

#[derive(Default)]
struct Cong {
    /// Chặn tới thời điểm này. None = không chặn.
    den: Option<Instant>,
    truoc_dong: bool,
}

impl Cong {
    fn trang_thai(&mut self, tts_dang_doc: bool, gio: Instant) -> Cua {
        if tts_dang_doc {
            self.den = Some(gio + NHA_CONG);
        }
        let dong = self.den.is_some_and(|t| gio < t);
        if !dong {
            self.den = None;
        }
        let cua = match (self.truoc_dong, dong) {
            (_, true) => Cua::Dong,
            (true, false) => Cua::VuaMo,
            (false, false) => Cua::Mo,
        };
        self.truoc_dong = dong;
        cua
    }
}

/// Nguồn tiếng đang mở. Chỉ để **giữ cho sống**: thả ra là đóng loopback, nên không đọc field
/// nào cả — cả hai bản đều dọn mình trong `Drop`.
#[allow(dead_code)]
enum Nguon {
    /// Đường TỐT: loopback loại trừ chính app, không cần bịt tai — xem [`crate::loopback`].
    Rieng(loopback::Tieng),
    /// Đường lùi: cpal nghe cả tiếng của chính app, nên **phải** bịt tai lúc TTS đọc.
    Chung(cpal::Stream),
}

/// Mở nguồn tiếng, ưu tiên đường loại trừ chính app.
///
/// Trả về `(nguồn, tần số, có phải bịt tai không)`. Máy cũ hơn Win10 20H1 thì lùi về cpal và
/// bịt tai như bản trước — mất chừng nửa số câu thoại, nhưng còn hơn không nghe được gì.
fn mo_nguon(tx: Sender<Vec<f32>>) -> Result<(Nguon, u32, bool), String> {
    match loopback::mo(tx.clone()) {
        Ok(t) => Ok((Nguon::Rieng(t), loopback::SR, false)),
        Err(e) => {
            eprintln!("[ASR] không bắt riêng được tiếng game ({e}) -> lùi về cpal + bịt tai");
            let (st, sr) = mo_loopback(tx)?;
            st.play().map_err(|e| format!("không bật được loopback: {e}"))?;
            Ok((Nguon::Chung(st), sr, true))
        }
    }
}

/// Mở loopback: thiết bị PHÁT mặc định, mở ở chế độ input. Trên Windows (WASAPI) đây chính là
/// cách bắt "mọi thứ đang phát ra loa" mà không cần Stereo Mix hay driver ảo.
///
/// **Chỉ còn là đường lùi.** Nó nghe cả tiếng của chính app nên kéo theo cả cái cổng bịt tai —
/// xem [`crate::loopback`] để biết vì sao chỗ đó đắt.
fn mo_loopback(tx: Sender<Vec<f32>>) -> Result<(cpal::Stream, u32), String> {
    let host = cpal::default_host();
    let dev = host
        .default_output_device()
        .ok_or("không thấy thiết bị phát nào để nghe ké")?;
    let sup: SupportedStreamConfig = dev
        .default_output_config()
        .map_err(|e| format!("không đọc được cấu hình thiết bị phát: {e}"))?;
    let sr = sup.sample_rate().0;
    let ch = sup.channels() as usize;
    let cfg: cpal::StreamConfig = sup.config();
    let loi = |e| eprintln!("[ASR] lỗi luồng âm thanh: {e:?}");
    // Trộn mọi kênh về mono — sherpa chỉ nhận mono, và thoại thường nằm ở kênh giữa/đều hai bên.
    let st = match sup.sample_format() {
        SampleFormat::F32 => dev.build_input_stream(
            &cfg,
            move |d: &[f32], _: &_| {
                if !d.is_empty() {
                    let _ = tx.send(d.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect());
                }
            },
            loi,
            None,
        ),
        SampleFormat::I16 => dev.build_input_stream(
            &cfg,
            move |d: &[i16], _: &_| {
                if !d.is_empty() {
                    let _ = tx.send(
                        d.chunks(ch)
                            .map(|f| {
                                f.iter().map(|&s| f32::from(s) / 32768.0).sum::<f32>() / ch as f32
                            })
                            .collect(),
                    );
                }
            },
            loi,
            None,
        ),
        k => return Err(format!("định dạng mẫu không hỗ trợ: {k:?}")),
    }
    .map_err(|e| format!("không mở được loopback: {e}"))?;
    Ok((st, sr))
}

/// Vét tiếng vừa bắt được vào `buf`, trả về **đỉnh biên độ của phần mới** — [`Ngong`] cần nó để
/// biết có ai đang phát tiếng hay không.
fn vet(rx: &Receiver<Vec<f32>>, buf: &mut Vec<f32>) -> f32 {
    let mut dinh = 0.0f32;
    while let Ok(v) = rx.try_recv() {
        dinh = dinh.max(v.iter().fold(0.0f32, |a, m| a.max(m.abs())));
        buf.extend_from_slice(&v);
    }
    dinh
}

/// Im lặng liên tục quá lâu thì nói cho người dùng biết. Dưới mức này coi như không có tiếng —
/// loopback lúc không ai phát trả về đúng số 0, để ngưỡng thấp cho tiếng nền rất khẽ vẫn tính.
const NGUONG_TIENG: f32 = 0.001;
const IM_LAU: Duration = Duration::from_secs(20);

/// Ghi một dòng biên độ vào nhật ký chẩn đoán mỗi ngần này. Đủ dày để đối chiếu với câu app đọc,
/// đủ thưa để không phình file.
const NHIP_GHI: Duration = Duration::from_secs(2);

/// Ngóng xem có nghe thấy gì không.
///
/// **Vì sao cần.** Nếu game xuất tiếng ở chế độ WASAPI độc quyền thì âm thanh đi thẳng xuống
/// driver, không qua bộ trộn, nên loopback — cả đường loại trừ tiến trình lẫn cpal — đều không
/// nghe thấy gì. Chọn sai thiết bị phát mặc định cũng ra y hệt. Không có chỗ này thì app hiện
/// "Đang nghe tiếng game" rồi ngồi im vĩnh viễn, người dùng không có lấy một manh mối.
///
/// **Phải trừ lúc app đang đọc.** Loopback đã loại tiếng của chính app nên trong suốt lúc TTS
/// nói, cái nghe được là số 0 thật — tính vào thì một đoạn thuyết minh dài sẽ bị báo nhầm.
struct Ngong {
    /// Lần cuối có tiếng (hoặc chính app đang nói).
    moc: Instant,
    /// Đã báo rồi thì đừng lải nhải mỗi nhịp.
    dang_bao: bool,
}

impl Ngong {
    fn moi(gio: Instant) -> Self {
        Self { moc: gio, dang_bao: false }
    }

    fn nhip(&mut self, dinh: f32, app_dang_doc: bool, gio: Instant, bao: &dyn Fn(String)) {
        if dinh > NGUONG_TIENG || app_dang_doc {
            self.moc = gio;
            if self.dang_bao {
                self.dang_bao = false;
                bao("Đã nghe thấy tiếng — nguồn âm thanh chạy bình thường.".into());
            }
        } else if !self.dang_bao && gio.duration_since(self.moc) >= IM_LAU {
            self.dang_bao = true;
            bao(format!(
                "Chưa nghe thấy tiếng nào suốt {}s — kiểm tra thiết bị phát mặc định, \
                 hoặc game đang xuất âm thanh ở chế độ độc quyền (WASAPI exclusive).",
                IM_LAU.as_secs()
            ));
        }
    }
}

/// Nạp `buf` vào bộ nhận dạng, mỗi câu chốt xong thì gọi `ra`.
///
/// **Không truyền tần số 16k**: sherpa nhận tần số thật của thiết bị rồi tự lấy mẫu lại bên
/// trong, nên khỏi phải tự viết bộ hạ mẫu (bản gốc RiTaiAI có `pcm.rs` chỉ vì phía cloud cần
/// đúng 16k để gửi lên mạng).
fn nap(
    rec: &OnlineRecognizer,
    st: &OnlineStream,
    buf: &mut Vec<f32>,
    sr: u32,
    may: &May,
    ra: &dyn Fn(String),
) {
    let n = chunk(sr);
    while buf.len() >= n {
        let m: Vec<f32> = buf.drain(..n).collect();
        st.accept_waveform(sr as i32, &m);
        // Giải mã CHO HẾT rồi mới ngó mốc ngắt — đúng thứ tự trong ví dụ C++ của sherpa.
        //
        // **Đây từng là lỗi thật.** Ban đầu tôi kiểm mốc ngắt NGAY TRONG vòng giải mã, nên
        // `reset` chạy giữa chừng và vứt luôn phần đã nạp mà chưa kịp lấy ra.
        //
        // Chỉ lấy câu ĐÃ CHỐT. Bản gốc RiTaiAI còn đẩy chữ tạm lên overlay cho người dùng
        // thấy máy đang nghe; ở đây câu chỉ đi thẳng vào khâu dịch nên chữ tạm vô dụng.
        while rec.is_ready(st) {
            rec.decode(st);
        }
        if rec.is_endpoint(st) {
            chot(rec, st, may, ra);
        }
    }
}

/// Lấy mẩu vừa chốt, thêm dấu câu, chẻ ra câu, **đẩy đi HẾT ngay**, rồi reset.
///
/// **Không giữ đuôi lại chờ.** Trước đây tôi cho cả bản chạy máy đi qua [`Gom`] — bộ gom giữ
/// đuôi câu tới 1,5s để chờ dấu chấm, nên MỌI câu bỏ lửng đều tới muộn thêm ngần ấy, nghe rõ
/// là chậm. Bản chạy máy đã có mốc ngắt riêng của bộ nhận dạng (khoảng lặng 0,8s) nên mốc đó
/// chính là ranh giới câu rồi, chờ thêm chẳng được gì. Chỉ bản đám mây mới cần gom, vì nó bắn
/// chữ về theo lô 1 giây bất kể câu đã hết hay chưa.
fn chot(rec: &OnlineRecognizer, st: &OnlineStream, may: &May, ra: &dyn Fn(String)) {
    let text = rec.get_result(st).map(|r| r.text).unwrap_or_default();
    let t = text.trim();
    if !t.is_empty() {
        nhat_ky::ghi("nghe", format_args!("(máy) {t:?}"));
    }
    // Bỏ mẩu rác chỉ có dấu câu — endpoint chốt trên đoạn cực ngắn hay ra token vụn.
    if t.chars().any(char::is_alphanumeric) {
        let (cau, con) = chia_cau(&may.dau_cau(t));
        for c in cau {
            ra(c);
        }
        if con.chars().any(char::is_alphanumeric) {
            ra(con);
        }
    }
    rec.reset(st);
}

/// Cái "não" nhận dạng. Hai bản: chạy máy ([`May`]) và Speechmatics ([`May Ngoai`](MayNgoai)).
///
/// Phần bắt loopback, cổng chặn tiếng của chính app và bộ gom câu nằm ngoài trait này vì cả hai
/// đều cần y hệt — chỉ khâu "tiếng ra chữ" là khác.
trait Nao {
    /// Nạp tiếng vừa nghe được. `buf` là bộ đệm dùng chung, tự rút phần đã xử lý.
    fn nghe(&mut self, buf: &mut Vec<f32>, sr: u32, ra: &dyn Fn(String)) -> Result<(), String>;
    /// Bấm dừng: chốt nốt những gì còn dở.
    fn xa(&mut self, buf: &mut Vec<f32>, sr: u32, ra: &dyn Fn(String));
    /// Gọi mỗi nhịp. Chỉ bản đám mây cần (nó có hạn chờ đuôi câu); bản chạy máy chốt ngay ở
    /// mỗi mốc ngắt nên không có gì để đợi.
    fn nhip(&mut self, _gio: Instant, _ra: &dyn Fn(String)) {}
    /// Cổng vừa mở lại sau khi app đọc xong — bỏ trạng thái dở dang đi.
    fn mo_lai(&mut self) {}
    /// Cổng ĐANG đóng: `buf` chứa tiếng của chính app, không được nhận dạng.
    ///
    /// Mặc định vứt luôn — bản chạy máy giữ trạng thái ngay trong tiến trình nên `mo_lai` dọn
    /// được, mà bỏ qua quãng này còn tiết kiệm CPU đúng lúc app đang bận đọc.
    fn cam(&mut self, buf: &mut Vec<f32>, _sr: u32, _ra: &dyn Fn(String)) -> Result<(), String> {
        buf.clear();
        Ok(())
    }
}

/// Bộ nhận dạng + bộ thêm dấu câu đi liền nhau.
struct May {
    rec: OnlineRecognizer,
    punct: Option<OnlinePunctuation>,
}

impl May {
    /// Bộ nhận dạng xuất chữ HOA và KHÔNG dấu câu. Model dấu câu vừa thêm `. ! ?` vừa viết hoa
    /// đầu câu, nên đưa chữ thường vào cho nó. Thiếu model thì tự viết hoa chữ đầu — câu sẽ dài
    /// lê thê vì `chia_cau` không có gì để cắt, nhưng vẫn đọc được.
    fn dau_cau(&self, t: &str) -> String {
        let thuong = t.to_lowercase();
        match &self.punct {
            Some(p) => p.add_punctuation(&thuong).unwrap_or(thuong),
            None => dep(&thuong),
        }
    }
}

/// Bản chạy máy: sherpa giữ stream riêng nên phải cầm luôn `st`.
struct MayLocal {
    may: May,
    st: OnlineStream,
}

impl Nao for MayLocal {
    fn nghe(&mut self, buf: &mut Vec<f32>, sr: u32, ra: &dyn Fn(String)) -> Result<(), String> {
        nap(&self.may.rec, &self.st, buf, sr, &self.may, ra);
        Ok(())
    }

    fn xa(&mut self, buf: &mut Vec<f32>, sr: u32, ra: &dyn Fn(String)) {
        xa(&self.st, buf, sr, &self.may, ra);
    }

    fn mo_lai(&mut self) {
        self.may.rec.reset(&self.st);
    }
}

/// Bản Speechmatics: gom đủ một lô rồi mới gửi, và rút chữ về ở mỗi nhịp.
///
/// KHÔNG cần model dấu câu — Speechmatics trả chữ đã có sẵn dấu, [`Gom`] cắt được ngay.
struct MayNgoai {
    phien: speechmatics::Phien,
    lo: Vec<f32>,
    /// Chỉ bản đám mây mới cần gom: nó bắn chữ về theo lô ~1 giây bất kể câu đã hết hay chưa,
    /// nên một câu thường tới làm nhiều mảnh.
    gom: Gom,
    /// Giữ lại để nối lại khi đứt.
    key: String,
    ngon_ngu: String,
    /// Còn được nối lại mấy lần nữa. Hết thì mới chịu thua — mạng chập chờn giữa lúc chơi là
    /// chuyện thường, đứt một cái mà tắt luôn tính năng thì quá đỏng đảnh.
    con_thu: u32,
}

/// Nối lại tối đa ngần này lần trong một phiên. RiTaiAI nối lại vô hạn (nó là trợ lý họp, chạy
/// cả buổi); ở đây chặn lại để key sai hoặc hết hạn mức không thành vòng lặp gọi mạng vô tận.
const THU_LAI: u32 = 5;

impl MayNgoai {
    /// Đứt giữa chừng thì dựng lại phiên. Tiếng trong lúc đứt coi như mất — Speechmatics tính
    /// giờ theo tiếng đã gửi, cố gửi bù đoạn cũ vừa tốn tiền vừa làm bản dịch lệch pha.
    fn noi_lai(&mut self, vi: &str) -> Result<(), String> {
        if self.con_thu == 0 {
            return Err(format!("{vi} — đã thử nối lại {THU_LAI} lần, chịu"));
        }
        self.con_thu -= 1;
        eprintln!("[ASR] Speechmatics đứt ({vi}), đang nối lại…");
        self.lo.clear();
        self.phien = speechmatics::Phien::mo(&self.key, &self.ngon_ngu)
            .map_err(|e| format!("nối lại Speechmatics không được: {e}"))?;
        Ok(())
    }
}

impl Nao for MayNgoai {
    fn nghe(&mut self, buf: &mut Vec<f32>, sr: u32, ra: &dyn Fn(String)) -> Result<(), String> {
        self.lo.append(buf);
        // Một lô 250ms tính THEO TẦN SỐ THIẾT BỊ, vì hạ mẫu diễn ra lúc gửi.
        let can = (sr as usize) / 4;
        while self.lo.len() >= can {
            let m: Vec<f32> = self.lo.drain(..can).collect();
            if let Err(e) = self.phien.gui(speechmatics::ha_mau(&m, sr)) {
                return self.noi_lai(&e);
            }
        }
        // Đẩy nốt phần còn kẹt trong đệm gửi từ nhịp trước.
        if let Err(e) = self.phien.day() {
            return self.noi_lai(&e);
        }
        let gio = Instant::now();
        match self.phien.nhan() {
            Ok((chu, dang_noi)) => {
                if dang_noi {
                    self.gom.con_noi(gio);
                }
                for c in chu {
                    nhat_ky::ghi("nghe", format_args!("(Speechmatics) {c:?}"));
                    self.gom.them(&c, gio, ra);
                }
                Ok(())
            }
            Err(e) => self.noi_lai(&e),
        }
    }

    /// **Gửi IM LẶNG chứ không bỏ trống.** Bản chạy máy vứt quãng này đi được, bản đám mây thì
    /// không: máy chủ dựng lại câu từ chính dòng tiếng nó nhận, cắt mất mấy giây ở giữa là hai
    /// câu cách nhau nửa phút bị dán liền nhau không có lấy một quãng nghỉ. Nó sẽ không chốt
    /// câu trước, rồi ghép luôn câu sau vào — đúng cái "bắt không chuẩn" thấy trên màn hình.
    ///
    /// Thay bằng đúng ngần ấy mẫu 0 thì dòng tiếng vẫn liên tục, máy chủ nghe thấy một quãng
    /// lặng thật và chốt câu đang dở. Đổi lại: quãng im lặng này vẫn tính vào giờ Speechmatics.
    /// Không tránh được — kết nối vẫn mở suốt lúc đó dù có gửi hay không.
    /// Câu máy chủ chốt được nhờ quãng lặng này vẫn phải đi tiếp (`ra`), không được nuốt: đó
    /// chính là câu thoại nói ngay trước lúc app cất tiếng đọc.
    fn cam(&mut self, buf: &mut Vec<f32>, sr: u32, ra: &dyn Fn(String)) -> Result<(), String> {
        buf.iter_mut().for_each(|m| *m = 0.0);
        self.nghe(buf, sr, ra)
    }

    fn xa(&mut self, _buf: &mut Vec<f32>, _sr: u32, ra: &dyn Fn(String)) {
        // Không cố rút nốt chữ từ máy chủ: bấm dừng là muốn im NGAY, chờ mạng thêm vài giây
        // rồi mới đọc thì phản tác dụng. Chỉ chốt phần đã nhận được.
        self.gom.xa(ra);
    }

    fn nhip(&mut self, gio: Instant, ra: &dyn Fn(String)) {
        self.gom.nhip(gio, ra);
    }
}

/// **Ghi lại hai kết luận sai đã tự sửa**, để không ai đi tìm lại từ đầu. Triệu chứng: file mẫu
/// ra "GOD" rồi nhảy thẳng tới "GIVEN HER…", nuốt gọn "AS A DIRECT CONSEQUENCE OF THE SIN WHICH
/// MAN THUS PUNISHED HAD".
///
/// 1. Tưởng `reset` ăn mất vế đó. Sai: bỏ HẲN mọi lời gọi `reset` vẫn ra đúng chuỗi thiếu y hệt.
/// 2. Kết luận tiếp "vậy là model không nghe ra, hết cách". Cũng chưa đúng — đúng ra là **model
///    ĐA NGỮ** không nghe ra. Đổi sang model chuyên tiếng Anh thì cả vế hiện đủ. Xem [`REPO`].
///
/// Chốt nốt câu đang dở dang. **Gọi khi dừng hẳn.**
///
/// Sherpa chỉ nhả câu ở mốc `is_endpoint`, tức là phải có quãng im đủ dài. Tiếng dứt đột ngột
/// thì không có mốc đó và chữ nằm kẹt trong stream — đo được trên file mẫu: câu 6,6s ra
/// **rỗng hoàn toàn**, câu 16,7s **mất nguyên vế cuối**. Trong game thì đó là câu thoại cuối
/// cùng trước khi người dùng bấm dừng: không xả là mất hẳn.
fn xa(st: &OnlineStream, buf: &mut Vec<f32>, sr: u32, may: &May, ra: &dyn Fn(String)) {
    if !buf.is_empty() {
        st.accept_waveform(sr as i32, buf);
        buf.clear();
    }
    // Đệm nửa giây im lặng TRƯỚC khi báo hết. Zipformer cần một quãng phía sau mới quyết được
    // từ cuối, `input_finished` không tự bù đủ nên chữ chót bị cụt: `8k.wav` ra
    // "…less with hope than appre", thêm đệm thì ra đủ "…than apprehension.".
    st.accept_waveform(sr as i32, &vec![0.0f32; sr as usize / 2]);
    st.input_finished();
    while may.rec.is_ready(st) {
        may.rec.decode(st);
    }
    chot(&may.rec, st, may, ra);
}

/// Model xuất chữ HOA hết. Để nguyên thì LLM dịch hay tưởng là hét, còn Google Translate thì
/// trả về bản dịch cũng viết hoa toàn bộ — đọc lên nghe không khác nhưng log thì chói mắt.
fn dep(t: &str) -> String {
    let mut c = t.to_lowercase();
    if let Some(d) = c.get_mut(..1) {
        d.make_ascii_uppercase();
    }
    c
}

/// Vòng đời nguồn âm thanh. **Phải gọi trong một thread riêng và ở lại đó**: `cpal::Stream` và
/// `OnlineStream` đều `!Send`, chúng phải sống và chết trong cùng thread đã dựng ra chúng.
///
/// `tts_dang_doc` là cờ dùng chung với [`crate::tts`] — xem [`Cong`].
pub fn run(
    engine: &str,
    key: &str,
    ngon_ngu: &str,
    stop: &AtomicBool,
    tts_dang_doc: &AtomicBool,
    ra: &dyn Fn(String),
    bao: &dyn Fn(String),
) -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<Vec<f32>>();
    let (nguon, sr, bit_tai) = mo_nguon(tx)?;
    let mut nao: Box<dyn Nao> = if engine == "speechmatics" {
        bao("Đang kết nối Speechmatics…".into());
        let phien = speechmatics::Phien::mo(key, ngon_ngu)?;
        bao(format!("Đang nghe tiếng game qua Speechmatics ({ngon_ngu})"));
        Box::new(MayNgoai {
            phien,
            lo: Vec::new(),
            gom: Gom::default(),
            key: key.to_string(),
            ngon_ngu: ngon_ngu.to_string(),
            con_thu: THU_LAI,
        })
    } else {
        let d = dir();
        let may = May { rec: setup(&d)?, punct: nap_dau_cau(&d) };
        let st = mo_stream(&may.rec, sr);
        bao(format!(
            "Đang nghe tiếng game ({sr} Hz) — nói tiếng Anh sẽ được dịch{}{}",
            if may.punct.is_some() { "" } else { " · THIẾU model dấu câu, câu sẽ rất dài" },
            if bit_tai { " · máy cũ: phải bịt tai lúc app đọc nên sẽ sót câu" } else { "" }
        ));
        Box::new(MayLocal { may, st })
    };

    let mut buf = Vec::<f32>::new();
    let mut cong = Cong::default();
    let mut ngong = Ngong::moi(Instant::now());
    let mut loi = None;
    // Đối chiếu "app đang đọc gì" với "lúc đó có tiếng thật không" — xem `crate::nhat_ky`.
    let mut nhip_ghi = Instant::now();
    let mut dinh_ky = 0.0f32;

    // Mỗi câu đẩy đi đều ghi lại, để biết khâu nào đẻ ra nó. Che luôn `ra` để không có đường
    // nào lỡ đi vòng qua chỗ ghi này.
    let ghi_ra = |s: String| {
        nhat_ky::ghi("cau", format_args!("{s:?}"));
        ra(s);
    };
    let ra: &dyn Fn(String) = &ghi_ra;

    nhat_ky::ghi(
        "mo",
        format_args!("nguồn={} {sr} Hz · bịt tai={bit_tai}", if engine == "speechmatics" {
            "Speechmatics"
        } else {
            "máy"
        }),
    );

    while !stop.load(Ordering::Relaxed) {
        let dinh = vet(&rx, &mut buf);
        let gio = Instant::now();
        let dang_doc = tts_dang_doc.load(Ordering::Relaxed);
        ngong.nhip(dinh, dang_doc, gio, bao);
        dinh_ky = dinh_ky.max(dinh);
        if gio.duration_since(nhip_ghi) >= NHIP_GHI {
            nhat_ky::ghi(
                "tieng",
                format_args!(
                    "đỉnh {dinh_ky:.4}{}",
                    if dang_doc { " · app đang đọc" } else { "" }
                ),
            );
            (nhip_ghi, dinh_ky) = (gio, 0.0);
        }
        // `bit_tai` sai (đường thường) -> cổng luôn mở, vì dòng tiếng đã không chứa tiếng của
        // app rồi. Chỉ máy phải lùi về cpal mới còn phải canh giờ.
        let tu_noi = bit_tai && dang_doc;
        match cong.trang_thai(tu_noi, gio) {
            // Tiếng của chính app — KHÔNG được nhận dạng, nếu không thì nghe mình đọc rồi dịch
            // rồi đọc lại, lặp vô hạn. Mỗi bộ não xử lý quãng này một kiểu, xem `Nao::cam`.
            Cua::Dong => {
                if let Err(e) = nao.cam(&mut buf, sr, ra) {
                    loi = Some(e);
                    break;
                }
            }
            Cua::VuaMo => nao.mo_lai(),
            Cua::Mo => {
                if let Err(e) = nao.nghe(&mut buf, sr, ra) {
                    loi = Some(e);
                    break;
                }
            }
        }
        // Chốt đuôi bỏ lửng (chỉ bản đám mây có). Chạy CẢ lúc cổng đóng: người nói dứt câu
        // xong app mới đọc, không chạy thì cái đuôi nằm chết trong bộ gom suốt lúc app nói.
        nao.nhip(gio, ra);
        std::thread::sleep(Duration::from_millis(5));
    }
    // Câu thoại cuối trước lúc bấm dừng vẫn phải được đọc — kể cả khi vừa đứt kết nối.
    nao.xa(&mut buf, sr, ra);
    // nguon drop ở đây -> đóng loopback.
    drop(nguon);
    match loi {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_f32(p: &str) -> (u32, Vec<f32>) {
        let b = std::fs::read(p).expect("đọc wav");
        let (sr, pcm) = crate::wave_out::split_wav(&b).expect("wav hợp lệ");
        (sr, pcm.chunks_exact(2).map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0).collect())
    }

    /// **Đo giá CPU trước khi tin vào tính năng này.** Khác OCR (chạy 16-73ms mỗi 700ms, có
    /// nghỉ), nhận dạng streaming phải nuốt 100% âm thanh liên tục — nếu nó ăn nửa nhân thì
    /// không dùng chung với game AAA được.
    ///
    ///     VN_WAV=<file.wav> cargo test --release --lib asr::tests::do_gia -- --ignored --nocapture
    ///
    /// `VN_MODEL_DIR` trỏ sang thư mục model khác để đọ hai model trên cùng file — đặt file
    /// trong đó đúng tên như [`FILES`] là chạy được.
    #[test]
    #[ignore]
    fn do_gia_cpu() {
        let (sr, mau) = wav_f32(&std::env::var("VN_WAV").unwrap());
        let giay = mau.len() as f32 / sr as f32;

        let d = std::env::var("VN_MODEL_DIR").map_or_else(|_| dir(), PathBuf::from);
        assert!(d.join(TOKENS).exists(), "chưa có model — chạy tai_model_that trước");
        let may = May { rec: setup(&d).unwrap(), punct: nap_dau_cau(&d) };
        assert!(may.punct.is_some(), "thiếu model dấu câu — câu sẽ không bao giờ được chẻ");
        let st = mo_stream(&may.rec, sr);
        let mut buf = mau;
        let nghe = std::cell::RefCell::new(String::new());
        let f = |s: String| {
            println!("  [mẩu] {s}");
            nghe.borrow_mut().push_str(&s);
            nghe.borrow_mut().push(' ');
        };

        let t = Instant::now();
        nap(&may.rec, &st, &mut buf, sr, &may, &f);
        xa(&st, &mut buf, sr, &may, &f);
        let chay = t.elapsed().as_secs_f32();

        println!(
            "tiếng {giay:.2}s · xử lý {chay:.2}s · RTF {:.3} = {:.1}% một nhân",
            chay / giay,
            100.0 * chay / giay
        );
        println!("nghe ra: {}", nghe.borrow().trim());
        assert!(!nghe.borrow().trim().is_empty(), "không nhận ra chữ nào");
        assert!(chay < giay, "chậm hơn thời gian thực -> không theo kịp tiếng game");
    }

    /// Tải THẬT từ trạng thái trắng rồi nhận dạng thử — chốt cả URL, cả kích thước ghim trong
    /// [`FILES`], cả việc model tải về dùng được. Xoá sạch thư mục nên chạy tay:
    ///     cargo test --release --lib asr::tests::tai_model -- --ignored --nocapture
    #[test]
    #[ignore]
    fn tai_model_that() {
        let d = dir();
        let _ = std::fs::remove_dir_all(&d);
        assert!(!ready(), "xoá rồi mà ready() vẫn true");
        // Giả bộ máy đã cài bản đa ngữ cũ: file 339 MB mang tên khác phải bị dọn, không được
        // nằm lại vĩnh viễn vì `ready()` chẳng bao giờ ngó tới nó.
        std::fs::create_dir_all(&d).unwrap();
        let cu = d.join("encoder-epoch-75-avg-11-chunk-16-left-128.int8.onnx");
        std::fs::write(&cu, b"model doi ngu cu").unwrap();

        ensure_model(&crate::runtime::im).expect("tải model");
        assert!(ready(), "tải xong mà ready() vẫn false — kích thước ghim trong FILES sai?");
        assert!(!cu.exists(), "model cũ vẫn nằm lại chiếm chỗ");
        setup(&d).expect("model tải về phải dựng được bộ nhận dạng");
    }

    #[test]
    fn chia_dung_cho_ket_cau() {
        let (c, con) = chia_cau("Look out. Something is behind you! Run");
        assert_eq!(c, ["Look out.", "Something is behind you!"]);
        assert_eq!(con, "Run", "đuôi dở dang phải giữ lại chờ mẩu sau");
        // Số thập phân KHÔNG phải ranh giới câu — sau dấu chấm là chữ số, không phải khoảng trắng.
        assert_eq!(chia_cau("It costs 3.14 gold").0, Vec::<String>::new());
        // Dấu đóng nháy đi kèm dấu kết thì thuộc về câu trước.
        assert_eq!(chia_cau("\"Go now!\" he said.").0, ["\"Go now!\"", "he said."]);
    }

    /// **Chốt đúng cái lỗi thấy trên màn hình thật.** Bộ nhận dạng chốt theo khoảng lặng nên bắn
    /// ra `But` rồi `Me to get away…` thành hai mẩu; dịch riêng thì mẩu đầu ra "Nhưng" cụt lủn.
    #[test]
    fn gom_manh_vun_thanh_cau() {
        let t0 = Instant::now();
        let ra = std::cell::RefCell::new(Vec::<String>::new());
        let f = |s: String| ra.borrow_mut().push(s);
        let mut g = Gom::default();

        g.them("But", t0, &f);
        assert!(ra.borrow().is_empty(), "mẩu chưa trọn câu mà đã đẩy đi dịch");
        g.them("me to get away from the information.", t0, &f);
        assert_eq!(*ra.borrow(), ["But me to get away from the information."]);

        // Người nói bỏ lửng: im đủ lâu thì phải chốt, không giữ mãi.
        g.them("And then i", t0, &f);
        g.nhip(t0 + IM_CHOT - Duration::from_millis(1), &f);
        assert_eq!(ra.borrow().len(), 1, "chốt sớm hơn hạn im lặng");
        g.nhip(t0 + IM_CHOT, &f);
        assert_eq!(*ra.borrow(), ["But me to get away from the information.", "And then i"]);
        // Chốt xong thì hạn phải tắt, đừng bắn lại chuỗi rỗng ở mọi nhịp sau đó.
        g.nhip(t0 + IM_CHOT * 9, &f);
        assert_eq!(ra.borrow().len(), 2);
    }

    /// Người nói ngập ngừng GIỮA câu thì không được chẻ đôi câu đó. Speechmatics chỉ gửi bản
    /// chốt khi có từ mới, nên chỗ ngập ngừng chẳng có bản chốt nào tới; nếu không nghe chữ tạm
    /// thì bộ gom hết giờ và ném nửa câu đi dịch.
    #[test]
    fn chu_tam_lui_han_chot() {
        let t0 = Instant::now();
        let ra = std::cell::RefCell::new(Vec::<String>::new());
        let f = |s: String| ra.borrow_mut().push(s);
        let mut g = Gom::default();

        g.them("I don't think", t0, &f);
        // Còn nghe thấy tiếng -> lùi hạn. Qua mốc IM_CHOT ban đầu vẫn phải im.
        for i in 1..=4 {
            g.con_noi(t0 + IM_CHOT / 2 * i);
            g.nhip(t0 + IM_CHOT / 2 * i, &f);
        }
        assert!(ra.borrow().is_empty(), "đang nói dở mà đã chẻ câu đem đi dịch");

        // Nói nốt -> ra trọn câu, không phải hai mảnh.
        let cuoi = t0 + IM_CHOT * 2;
        g.them("we should go.", cuoi, &f);
        assert_eq!(*ra.borrow(), ["I don't think we should go."]);

        // Dứt hẳn thì hạn tính từ lần lùi cuối, vẫn phải chốt được đuôi.
        g.them("Wait", cuoi, &f);
        g.con_noi(cuoi);
        g.nhip(cuoi + IM_CHOT, &f);
        assert_eq!(ra.borrow().len(), 2, "hết tiếng rồi mà đuôi câu vẫn kẹt trong bộ gom");

        // Bộ gom RỖNG thì chữ tạm không được tự dựng hạn — chốt ra chuỗi rỗng chẳng để làm gì.
        g.con_noi(cuoi);
        assert!(g.han.is_none(), "bộ gom rỗng mà chữ tạm vẫn dựng hạn chốt");
    }

    /// Báo im lặng phải đúng lúc: không lải nhải, không báo nhầm lúc app đang đọc, và tự rút
    /// lại khi tiếng quay về.
    #[test]
    fn bao_khi_im_lang_qua_lau() {
        let t0 = Instant::now();
        let ra = std::cell::RefCell::new(Vec::<String>::new());
        let f = |s: String| ra.borrow_mut().push(s);
        let mut n = Ngong::moi(t0);

        // Chưa tới hạn thì im.
        n.nhip(0.0, false, t0 + IM_LAU - Duration::from_millis(1), &f);
        assert!(ra.borrow().is_empty(), "báo sớm hơn hạn");

        n.nhip(0.0, false, t0 + IM_LAU, &f);
        assert_eq!(ra.borrow().len(), 1, "tới hạn mà không báo");
        assert!(ra.borrow()[0].contains("độc quyền"), "báo không nói cách xử lý");

        // Đã báo rồi thì thôi, đừng bắn lại mỗi nhịp 5ms.
        for i in 1..50 {
            n.nhip(0.0, false, t0 + IM_LAU + Duration::from_millis(i * 5), &f);
        }
        assert_eq!(ra.borrow().len(), 1, "báo lải nhải");

        // Tiếng quay lại -> rút lại lời báo.
        let sau = t0 + IM_LAU * 2;
        n.nhip(0.5, false, sau, &f);
        assert_eq!(ra.borrow().len(), 2);
        assert!(ra.borrow()[1].contains("Đã nghe thấy"), "không rút lại lời báo");
    }

    /// **Chốt đúng cái bẫy của đường bắt tiếng mới.** Loopback đã loại tiếng của chính app, nên
    /// suốt lúc TTS đọc thì cái nghe được là số 0 THẬT. Tính vào là một đoạn thuyết minh dài
    /// hơn 20 giây sẽ bị báo "không nghe thấy gì" trong khi mọi thứ vẫn chạy đúng.
    #[test]
    fn app_dang_doc_thi_khong_tinh_la_im_lang() {
        let t0 = Instant::now();
        let ra = std::cell::RefCell::new(Vec::<String>::new());
        let f = |s: String| ra.borrow_mut().push(s);
        let mut n = Ngong::moi(t0);

        // Đọc liên tục gấp ba lần hạn, tiếng bắt được là 0 tuyệt đối.
        for i in 1..=(IM_LAU.as_secs() * 3) {
            n.nhip(0.0, true, t0 + Duration::from_secs(i), &f);
        }
        assert!(ra.borrow().is_empty(), "báo nhầm trong lúc app đang đọc");

        // Đọc xong mà vẫn im thì mới tính, và tính TỪ LÚC đọc xong.
        let xong = t0 + Duration::from_secs(IM_LAU.as_secs() * 3);
        n.nhip(0.0, false, xong + IM_LAU - Duration::from_millis(1), &f);
        assert!(ra.borrow().is_empty(), "tính hạn từ trước lúc đọc xong");
        n.nhip(0.0, false, xong + IM_LAU, &f);
        assert_eq!(ra.borrow().len(), 1);
    }

    /// Chẩn đoán: in vị trí (giây) và chữ THÔ tại mỗi mốc ngắt, không qua model dấu câu.
    ///     VN_WAV=<file> cargo test --release --lib asr::tests::soi_moc -- --ignored --nocapture
    #[test]
    #[ignore]
    fn soi_moc_ngat() {
        let (sr, mau) = wav_f32(&std::env::var("VN_WAV").unwrap());
        let rec = setup(&dir()).unwrap();
        let st = mo_stream(&rec, sr);
        let mut vt = 0usize;
        for m in mau.chunks(chunk(sr)) {
            st.accept_waveform(sr as i32, m);
            vt += m.len();
            while rec.is_ready(&st) {
                rec.decode(&st);
            }
            if rec.is_endpoint(&st) {
                let t = rec.get_result(&st).map(|r| r.text).unwrap_or_default();
                println!("  {:5.2}s  NGAT  {:?}", vt as f32 / sr as f32, t.trim());
                rec.reset(&st);
            }
        }
        st.input_finished();
        while rec.is_ready(&st) {
            rec.decode(&st);
        }
        let t = rec.get_result(&st).map(|r| r.text).unwrap_or_default();
        println!("  {:5.2}s  XA    {:?}", vt as f32 / sr as f32, t.trim());
    }

    /// Cổng phải đóng suốt lúc TTS đọc, giữ thêm khoảng nhả, rồi báo VuaMo ĐÚNG MỘT LẦN.
    /// Báo nhiều lần thì reset liên tục, nuốt mất câu; không báo lần nào thì nửa câu dở trước
    /// khi đóng cổng ghép vào tiếng mới thành câu vô nghĩa.
    #[test]
    fn cong_chan_het_tieng_cua_chinh_app() {
        let t0 = Instant::now();
        let mut c = Cong::default();
        assert_eq!(c.trang_thai(false, t0), Cua::Mo);
        assert_eq!(c.trang_thai(true, t0), Cua::Dong, "TTS đang đọc mà không chặn");
        // Ngay khi TTS báo xong, cổng VẪN đóng cho hết khoảng nhả.
        let vua_xong = t0 + Duration::from_millis(10);
        assert_eq!(c.trang_thai(false, vua_xong), Cua::Dong, "nhả cổng quá sớm, đuôi tiếng lọt vào");
        assert_eq!(c.trang_thai(false, vua_xong + NHA_CONG), Cua::VuaMo);
        assert_eq!(c.trang_thai(false, vua_xong + NHA_CONG), Cua::Mo, "VuaMo báo quá một lần");
    }
}

