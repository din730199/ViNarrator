//! Nguồn **âm thanh**: nghe tiếng game ở loopback rồi nhận dạng tại chỗ (sherpa-onnx streaming
//! zipformer). Dành cho game KHÔNG có phụ đề, hoặc phụ đề nằm ngoài vùng quét.
//!
//! Chạy hoàn toàn offline, không key, không quota. Câu nhận được đi vào đúng đường dịch + đọc
//! của nguồn màn hình — xem [`crate::narrator`].
//!
//! Hai bộ não để chọn: **chạy máy** (model EN 20M int8, 41 MB + 7,6 MB dấu câu — chỉ tiếng
//! Anh, không cần mạng) và **Speechmatics** (đám mây, cần API key, nhiều thứ tiếng, chữ trả về
//! đã có sẵn dấu câu). Phần bắt loopback, cổng chặn và gom câu dùng chung — xem [`Nao`].
//!
//! **Vì sao không có khử vọng (AEC).** Bản gốc mà kiến trúc này bê về (RiTaiAI) bắt HAI luồng —
//! mic và loopback — nên phải khử tiếng loa lọt vào mic. Ở đây không có mic, chỉ loopback, nên
//! bỏ được cả `sonora` lẫn khâu đồng bộ khung 10ms.
//!
//! **Cái phải xử lý thay vào đó: app tự nghe chính nó.** ViNarrator phát tiếng Việt ra loa, mà
//! loopback nghe tất cả những gì loa phát — kể cả tiếng của chính nó. Không chặn thì nó nhận
//! dạng giọng đọc của mình, dịch, rồi đọc lại, lặp vô hạn. Xem [`Cong`].

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
use crate::speechmatics;

/// Model đa ngữ `streaming-zipformer-multi-2025-02-10` (encoder+joiner int8, decoder fp32).
///
/// **Đổi từ bản EN 20M sang bản này vì bản nhỏ nghe không đủ chuẩn trên tiếng game thật.** Bản
/// 20M chỉ 41 MB và rẻ hơn nhiều, nhưng nó là model streaming tiếng Anh NHỎ NHẤT mà sherpa
/// phát hành — nghe giọng đọc sách sạch thì được, nghe thoại lẫn nhạc nền thì không.
///
/// Kèm theo: bản này nghe được **nhiều thứ tiếng** (token phủ Latin + CJK + Kana + Cyrillic +
/// Thai + Arabic), nên chế độ Trung→Việt cũng dùng được nguồn âm thanh.
///
/// **Nguồn tải là bản re-host của RiTaiAI.** Không tìm ra bản gốc trên release chính thức của
/// k2-fsa lẫn HuggingFace (đã dò một loạt tên, 404/401 hết). Đây là điểm yếu thật: repo đó là
/// của cá nhân, họ xoá release là ViNarrator hết tải được model. Tìm được bản gốc thì đổi ngay.
const REPO: &str = "https://github.com/din730199/ritai-releases/releases/download/models";
const ENC: &str = "encoder-epoch-75-avg-11-chunk-16-left-128.int8.onnx";
const DEC: &str = "decoder-epoch-75-avg-11-chunk-16-left-128.onnx";
const JOIN: &str = "joiner-epoch-75-avg-11-chunk-16-left-128.int8.onnx";
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

/// Kèm kích thước THẬT (đã đối chiếu tarball chính thức với HuggingFace, khớp từng byte).
/// Dùng để bắt file tải dở: `runtime::fetch` ghi `.part` rồi mới đổi tên, nhưng nếu server cắt
/// giữa chừng mà không báo lỗi thì vẫn ra một file ngắn mang tên đúng.
const FILES: [(&str, u64); 4] =
    [(ENC, 296_583_597), (DEC, 33_837_085), (JOIN, 8_257_421), (TOKENS, 195_244)];
const PUNCT_FILES: [(&str, u64); 2] = [(PUNCT_MODEL, 7_490_500), (PUNCT_VOCAB, 149_430)];

/// Đuôi câu chưa có dấu kết mà im lặng quá ngần này thì chốt luôn, đừng giữ mãi trong bộ gom.
/// Nghe tiếng người nói ngập ngừng ("But…", "I…") sinh ra rất nhiều mẩu một từ; giữ lâu quá thì
/// bản dịch tới muộn, chốt sớm quá thì lại chẻ vụn đúng cái tật đang phải sửa.
const IM_CHOT: Duration = Duration::from_millis(1500);

/// Nạp sherpa theo từng mẩu ~67ms. Nhỏ hơn thì tốn phí gọi hàm, lớn hơn thì trễ chữ.
const CHUNK: usize = 3200;

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

/// Tải model (~41 MB). Gọi lại khi đã đủ thì trả về ngay.
pub fn ensure_model(bao: runtime::BaoTien) -> Result<(), String> {
    if ready() {
        return Ok(());
    }
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    runtime::du_dia(&d, 200)?;
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
    c.decoding_method = Some("greedy_search".into());
    // Ngắt câu. Đường CHÍNH là rule2: im lặng 0,8s sau khi đã nói -> chốt. Thoại game gần như
    // lúc nào cũng có nghỉ giữa các câu nên đường này gánh hầu hết.
    //
    // rule3 là lưới an toàn cho lúc nói một hơi không nghỉ. Tôi từng nới lên 12s để tránh nó
    // cắt giữa từ, nhưng như thế một đoạn độc thoại phải chờ tới 12 giây mới nghe được bản
    // dịch — chậm rõ rệt khi chơi. Về đúng 6s như RiTaiAI: thà thỉnh thoảng lệch một từ ở chỗ
    // nối còn hơn ngồi đợi.
    c.enable_endpoint = true;
    // rule1 (im lặng mà CHƯA ra chữ nào) TẮT hẳn, khác RiTaiAI để 1,6s. Sau mỗi  model
    // này câm chừng 2 giây để dựng lại 128 khung ngữ cảnh trái, và rule1 tưởng đó là im lặng
    // nên chốt ra chuỗi rỗng rồi reset lần nữa — càng reset càng câm. Đo trên file mẫu:
    //   để 1,6s -> 1.80s "GOD" · 3.80s "" · 9.80s "GIVEN HER…"  (mất một vế)
    //   tắt      -> 1.80s "GOD" · 7.80s "GIVEN HER…"             (hết mốc rỗng)
    // App này không cần chốt lúc chưa ai nói, nên rule1 chẳng có việc gì để làm.
    c.rule1_min_trailing_silence = 100.0;
    c.rule2_min_trailing_silence = 0.8;
    c.rule3_min_utterance_length = 6.0;
    OnlineRecognizer::create(&c)
        .ok_or_else(|| "không dựng được bộ nhận dạng — model hỏng hoặc tải dở".into())
}

/// Dựng stream rồi **mồi bằng một quãng im lặng** trước khi cho nghe tiếng thật.
///
/// **Đo được, không phải phòng xa.** Zipformer khởi động với bộ đệm ngữ cảnh trái bằng 0 nên
/// nuốt mất đoạn đầu. Cùng một file mẫu, mọi thứ khác giữ nguyên:
///
/// | | Nghe ra |
/// |---|---|
/// | không mồi | `The yellow lamps would light up…` |
/// | mồi 1s | `After early nightfall the yellow lamps would light up…` |
///
/// Sự thật là `AFTER EARLY NIGHTFALL THE YELLOW LAMPS…` — không mồi thì **mất trắng 2 giây
/// đầu**, và trong game đó thường là chỗ mở câu thoại.
///
/// Đã loại trừ hết nghi phạm khác trước khi kết luận: nạp nguyên khối một lần (đúng ví dụ của
/// crate) và tắt hẳn `enable_endpoint` đều cho kết quả y hệt lúc chưa mồi.
///
/// **Phải mồi lại sau MỖI lần `reset`, không chỉ lúc dựng stream.** Tôi từng kết luận ngược
/// lại dựa trên model EN 20M — sai với model đa ngữ đang dùng, vì nó mang 128 khung ngữ cảnh
/// trái (`chunk-16-left-128`) và `reset` xoá sạch chỗ đó.
///
/// Soi mốc ngắt trên file mẫu 16,7s lúc chưa mồi lại thấy rõ:
///
/// ```text
///  1.80s  NGAT  "GOD"
///  3.80s  NGAT  ""          <- hai giây liền không ra chữ nào, dù chỗ đó CÓ tiếng
///  9.80s  NGAT  "GIVEN HER A LOVELY CHILD…"
/// ```
///
/// Mất trắng "AS A DIRECT CONSEQUENCE OF THE SIN WHICH MAN THUS PUNISHED HAD". Sau `reset` bộ
/// nhận dạng câm chừng 2 giây để dựng lại ngữ cảnh, `rule1` tưởng là im lặng nên chốt ra chuỗi
/// rỗng rồi `reset` lần nữa.
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

/// Mở loopback: thiết bị PHÁT mặc định, mở ở chế độ input. Trên Windows (WASAPI) đây chính là
/// cách bắt "mọi thứ đang phát ra loa" mà không cần Stereo Mix hay driver ảo.
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

fn vet(rx: &Receiver<Vec<f32>>, buf: &mut Vec<f32>) {
    while let Ok(v) = rx.try_recv() {
        buf.extend_from_slice(&v);
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
    while buf.len() >= CHUNK {
        let m: Vec<f32> = buf.drain(..CHUNK).collect();
        st.accept_waveform(sr as i32, &m);
        // Giải mã CHO HẾT rồi mới ngó mốc ngắt — đúng thứ tự trong ví dụ C++ của sherpa.
        //
        // **Đây từng là lỗi thật.** Ban đầu tôi kiểm mốc ngắt NGAY TRONG vòng giải mã, nên
        // `reset` chạy giữa chừng và vứt luôn phần đã nạp mà chưa kịp lấy ra. Với model nhỏ
        // hầu như không thấy, nhưng đổi sang model lớn thì mất trắng cả vế: câu mẫu ra
        // "God" rồi nhảy thẳng tới "Given her a lovely child…", nuốt gọn đoạn "as a direct
        // consequence of the sin which man thus punished had" ở giữa.
        //
        // Chỉ lấy câu ĐÃ CHỐT. Bản gốc RiTaiAI còn đẩy chữ tạm lên overlay cho người dùng
        // thấy máy đang nghe; ở đây câu chỉ đi thẳng vào khâu dịch nên chữ tạm vô dụng.
        while rec.is_ready(st) {
            rec.decode(st);
        }
        if rec.is_endpoint(st) {
            chot(rec, st, sr, may, ra);
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
fn chot(rec: &OnlineRecognizer, st: &OnlineStream, sr: u32, may: &May, ra: &dyn Fn(String)) {
    let text = rec.get_result(st).map(|r| r.text).unwrap_or_default();
    let t = text.trim();
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
    sr: u32,
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
}

impl Nao for MayNgoai {
    fn nghe(&mut self, buf: &mut Vec<f32>, sr: u32, ra: &dyn Fn(String)) -> Result<(), String> {
        self.lo.append(buf);
        // Một lô 250ms tính THEO TẦN SỐ THIẾT BỊ, vì hạ mẫu diễn ra lúc gửi.
        let can = (sr as usize) / 4;
        while self.lo.len() >= can {
            let m: Vec<f32> = self.lo.drain(..can).collect();
            self.phien.gui(speechmatics::ha_mau(&m, sr))?;
        }
        let gio = Instant::now();
        for c in self.phien.nhan()? {
            self.gom.them(&c, gio, ra);
        }
        Ok(())
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
    st.input_finished();
    while may.rec.is_ready(st) {
        may.rec.decode(st);
    }
    chot(&may.rec, st, sr, may, ra);
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
    let (stream, sr) = mo_loopback(tx)?;
    let mut nao: Box<dyn Nao> = if engine == "speechmatics" {
        bao("Đang kết nối Speechmatics…".into());
        let phien = speechmatics::Phien::mo(key, ngon_ngu)?;
        bao(format!("Đang nghe tiếng game qua Speechmatics ({ngon_ngu})"));
        Box::new(MayNgoai { phien, lo: Vec::new(), gom: Gom::default() })
    } else {
        let d = dir();
        let may = May { rec: setup(&d)?, punct: nap_dau_cau(&d) };
        let st = mo_stream(&may.rec, sr);
        bao(format!(
            "Đang nghe tiếng game ({sr} Hz) — nói tiếng Anh sẽ được dịch{}",
            if may.punct.is_some() { "" } else { " · THIẾU model dấu câu, câu sẽ rất dài" }
        ));
        Box::new(MayLocal { may, st, sr })
    };
    stream.play().map_err(|e| format!("không bật được loopback: {e}"))?;

    let mut buf = Vec::<f32>::new();
    let mut cong = Cong::default();
    let mut loi = None;

    while !stop.load(Ordering::Relaxed) {
        vet(&rx, &mut buf);
        let gio = Instant::now();
        match cong.trang_thai(tts_dang_doc.load(Ordering::Relaxed), gio) {
            // Vứt tiếng của chính app đi, KHÔNG nạp vào bộ nhận dạng.
            Cua::Dong => buf.clear(),
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
    // stream drop ở đây -> đóng loopback.
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
    #[test]
    #[ignore]
    fn do_gia_cpu() {
        assert!(ready(), "chưa có model — chạy tai_model_that trước");
        let (sr, mau) = wav_f32(&std::env::var("VN_WAV").unwrap());
        let giay = mau.len() as f32 / sr as f32;

        let d = dir();
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
        ensure_model(&crate::runtime::im).expect("tải model");
        assert!(ready(), "tải xong mà ready() vẫn false — kích thước ghim trong FILES sai?");
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

    /// Chẩn đoán: in vị trí (giây) và chữ THÔ tại mỗi mốc ngắt, không qua model dấu câu.
    ///     VN_WAV=<file> cargo test --release --lib asr::tests::soi_moc -- --ignored --nocapture
    #[test]
    #[ignore]
    fn soi_moc_ngat() {
        let (sr, mau) = wav_f32(&std::env::var("VN_WAV").unwrap());
        let rec = setup(&dir()).unwrap();
        let st = mo_stream(&rec, sr);
        let mut vt = 0usize;
        for m in mau.chunks(CHUNK) {
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

