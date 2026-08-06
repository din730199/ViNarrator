//! Đổi tốc độ đọc mà **không đổi cao độ** — WSOLA (xếp chồng cửa sổ có dò điểm khớp).
//!
//! VieNeu không có núm tốc độ. Chữ ký thật của nó:
//! `infer_stream(text, ref_audio, voice, style, denoise, use_ref_codes, temperature, top_k,
//! top_p, max_new_frames, repetition_penalty, max_chars, apply_watermark)` — không tham số nào
//! chỉnh nhịp đọc, vì tốc độ nằm sẵn trong chuỗi token mà model sinh ra. Nên phải kéo/giãn ở
//! khâu tiếng, sau khi model đã nói xong.
//!
//! **Vì sao không đổi tần số phát** (`waveOutOpen(sr * speed)`, đúng một dòng): nó kéo luôn cao
//! độ. 1,5x là giọng cao thêm ~7 nửa cung — nghe như chuột. WSOLA cắt tiếng thành cửa sổ chồng
//! nhau rồi xếp lại thưa/dày hơn nên chu kỳ dao động giữ nguyên, tức cao độ giữ nguyên.
//!
//! **Có trạng thái, cố ý.** VieNeu đẩy tiếng theo mẩu (~9 mẩu/câu). Giãn từng mẩu riêng rẽ sẽ
//! để lại một chỗ gãy ở MỖI ranh giới mẩu — đúng loại lỗi đã phải sửa ở [`crate::wave_out`].
//! Bộ đệm ở đây bắc cầu qua ranh giới: chia mẩu kiểu gì cũng ra đúng một kết quả (có test).
//!
//! Vào/ra đều là PCM 16-bit little-endian dạng byte, khớp với [`crate::wave_out::split_wav`].

/// Cửa sổ ~32ms: đủ dài để ôm trọn một chu kỳ thanh quản của giọng trầm (~85 Hz = 12ms), đủ
/// ngắn để tai không nghe ra tiếng vọng. Làm chẵn để chia đôi được.
fn cua_so(sr: u32) -> usize {
    (sr as usize / 32) & !1
}

/// Dò điểm khớp trong ±10ms. Rộng hơn một chu kỳ thanh quản là thừa — chỉ tốn CPU.
fn tam_do(sr: u32) -> usize {
    sr as usize / 100
}

/// Lúc dò khớp chỉ lấy 1 trong 4 mẫu: rẻ đi 4 lần mà điểm khớp chọn ra gần như không đổi
/// (tiếng nói không có thành phần nào đủ cao tần để 4 mẫu bỏ qua làm lệch tương quan).
const BUOC_DO: usize = 4;

/// Dưới ngưỡng này coi như tốc độ 1x — khỏi tốn CPU cho thứ không ai nghe ra.
const NGUONG: f64 = 0.01;

pub fn can_keo(speed: f64) -> bool {
    (speed - 1.0).abs() > NGUONG
}

pub struct Stretch {
    /// >1 = đọc nhanh hơn = tiếng ra NGẮN đi theo đúng tỉ lệ này.
    ty_le: f32,
    w: usize,
    tam: usize,
    /// Cửa sổ Hann dựng sẵn. Hai nửa chồng nhau cộng lại đúng bằng 1 nên không gợn biên độ.
    han: Vec<f32>,
    /// Mẫu vào chưa dùng hết. Luôn giữ ít nhất `tam` mẫu phía trước `doc` để còn dò lùi.
    dem: Vec<i16>,
    /// Vị trí đọc danh nghĩa của cửa sổ kế tiếp. Để f32 vì bước nhảy `h * ty_le` là số lẻ —
    /// làm tròn mỗi bước sẽ trôi dần thành sai độ dài.
    doc: f32,
    /// Nửa sau của cửa sổ trước, đã nhân Hann, đang chờ cộng chồng với nửa đầu cửa sổ tới.
    duoi: Vec<f32>,
    /// Đoạn tiếng mà cửa sổ trước "đáng lẽ" phát tiếp. Dùng làm mẫu dò: chọn cửa sổ tới sao
    /// cho nối vào nghe liền nhất. Bỏ bước này (thành OLA thuần) là giọng bị ọp ẹp như vọng.
    mau: Vec<i16>,
    dau_tien: bool,
}

impl Stretch {
    pub fn new(sr: u32, ty_le: f32) -> Self {
        let w = cua_so(sr);
        let han = (0..w)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / w as f32).cos())
            .collect();
        Self {
            ty_le,
            w,
            tam: tam_do(sr),
            han,
            dem: Vec::new(),
            doc: 0.0,
            duoi: vec![0.0; w / 2],
            mau: Vec::new(),
            dau_tien: true,
        }
    }

    /// Đẩy một mẩu vào, lấy ra phần đã giãn xong. Ra ít hơn vào là bình thường — phần đuôi
    /// còn nằm trong đệm chờ mẩu sau, hoặc chờ [`Self::flush`].
    pub fn push(&mut self, pcm: &[u8], ra: &mut Vec<u8>) {
        self.dem.extend(pcm.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])));
        let (w, h) = (self.w, self.w / 2);

        while (self.doc as usize) + w + self.tam < self.dem.len() {
            let k = self.khop(self.doc as usize);
            for i in 0..h {
                let x = f32::from(self.dem[k + i]) * self.han[i];
                // Cửa sổ ĐẦU không có gì đứng trước để cộng chồng: lấy thẳng mẫu gốc, đừng
                // nhân Hann — nhân vào là câu nào cũng mở đầu bằng 16ms to dần.
                let y = if self.dau_tien { f32::from(self.dem[k + i]) } else { self.duoi[i] + x };
                ra.extend_from_slice(&kep(y).to_le_bytes());
            }
            for i in 0..h {
                self.duoi[i] = f32::from(self.dem[k + h + i]) * self.han[h + i];
            }
            self.mau.clear();
            self.mau.extend_from_slice(&self.dem[k + h..k + w]);
            self.dau_tien = false;

            self.doc += h as f32 * self.ty_le;
            // Cắt bớt đầu đệm, chừa đúng `tam` mẫu để lần dò sau còn lùi được. Không cắt thì
            // câu dài làm đệm phình mãi.
            if self.doc as usize > 4 * self.tam {
                let bo = self.doc as usize - self.tam;
                self.dem.drain(..bo);
                self.doc -= bo as f32;
            }
        }
    }

    /// Xả nốt đuôi câu. **Bắt buộc gọi**: sau cửa sổ cuối vẫn còn tới ~50ms nằm trong đệm —
    /// bỏ đi là mất phụ âm cuối của mỗi câu.
    ///
    /// Đuôi này phát ở 1x chứ không giãn (chưa đủ mẫu cho một cửa sổ nữa). Lệch ~50ms trên
    /// một câu vài giây thì không ai nghe ra, còn nuốt mất chữ thì nghe ra ngay.
    pub fn flush(&mut self, ra: &mut Vec<u8>) {
        let d = self.doc as usize;
        if self.dau_tien {
            // Câu ngắn hơn một cửa sổ: chưa phát gì cả, trả nguyên văn.
            for &s in &self.dem {
                ra.extend_from_slice(&s.to_le_bytes());
            }
        } else {
            // Thêm đúng một lượt cộng chồng tại chỗ (không tiến `doc`) để `duoi` tắt dần vào
            // tiếng thật thay vì tắt vào im lặng -> không có tiếng "tách".
            for i in 0..self.duoi.len() {
                let x = self.dem.get(d + i).copied().map_or(0.0, f32::from) * self.han[i];
                ra.extend_from_slice(&kep(self.duoi[i] + x).to_le_bytes());
            }
            if let Some(con) = self.dem.get(d + self.duoi.len()..) {
                for &s in con {
                    ra.extend_from_slice(&s.to_le_bytes());
                }
            }
        }
        self.dem.clear();
        self.doc = 0.0;
        self.duoi.fill(0.0);
        self.mau.clear();
        self.dau_tien = true;
    }

    /// Dò trong ±`tam` quanh `goc` xem cửa sổ nào nối vào `mau` nghe liền nhất (tương quan lớn
    /// nhất). Đây chính là chữ "W" của WSOLA.
    ///
    /// Dùng tương quan thô, không chuẩn hoá: trong khoảng ±10ms năng lượng tiếng nói gần như
    /// không đổi nên chuẩn hoá chỉ thêm một phép căn mỗi vị trí mà không đổi kết quả.
    fn khop(&self, goc: usize) -> usize {
        if self.mau.is_empty() {
            return goc;
        }
        let lo = goc.saturating_sub(self.tam);
        let hi = goc + self.tam;
        let mut tot = (f32::NEG_INFINITY, goc);
        for k in lo..=hi {
            let mut s = 0.0f32;
            for i in (0..self.mau.len()).step_by(BUOC_DO) {
                s += f32::from(self.mau[i]) * f32::from(self.dem[k + i]);
            }
            if s > tot.0 {
                tot = (s, k);
            }
        }
        tot.1
    }
}

fn kep(x: f32) -> i16 {
    x.clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn sin(hz: f32, giay: f32) -> Vec<u8> {
        let n = (SR as f32 * giay) as usize;
        (0..n)
            .flat_map(|i| {
                let x = 8000.0 * (std::f32::consts::TAU * hz * i as f32 / SR as f32).sin();
                (x as i16).to_le_bytes()
            })
            .collect()
    }

    /// `manh` = số mẫu mỗi lần đẩy vào. `usize::MAX` = đẩy hết một lần.
    fn keo(vao: &[u8], ty_le: f32, manh: usize) -> Vec<u8> {
        let mut s = Stretch::new(SR, ty_le);
        let mut ra = Vec::new();
        for m in vao.chunks(manh.saturating_mul(2).min(vao.len()).max(1)) {
            s.push(m, &mut ra);
        }
        s.flush(&mut ra);
        ra
    }

    fn mau(b: &[u8]) -> Vec<i16> {
        b.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
    }

    /// Ước lượng tần số bằng số lần đổi dấu — đủ chính xác cho sóng sin thuần.
    fn hz(b: &[u8]) -> f32 {
        let m = mau(b);
        let dao = m.windows(2).filter(|w| (w[0] >= 0) != (w[1] >= 0)).count();
        dao as f32 / 2.0 / (m.len() as f32 / SR as f32)
    }

    /// **Đây là test phân biệt WSOLA với phương án "đổi tần số phát".** Đổi tần số phát cũng
    /// cho đúng độ dài, nhưng 440 Hz sẽ thành 660 Hz. Ở đây cao độ phải GIỮ NGUYÊN.
    #[test]
    fn nhanh_hon_nhung_cao_do_khong_doi() {
        let vao = sin(440.0, 1.0);
        let ra = keo(&vao, 1.5, usize::MAX);
        let ty = mau(&vao).len() as f32 / mau(&ra).len() as f32;
        assert!((ty - 1.5).abs() < 0.05, "tỉ lệ ra {ty:.3}, đáng lẽ 1.5");
        let f = hz(&ra);
        assert!((f - 440.0).abs() < 20.0, "cao độ thành {f:.0} Hz — đang kéo cả cao độ");
    }

    #[test]
    fn cham_hon_thi_dai_ra() {
        let vao = sin(220.0, 1.0);
        let ra = keo(&vao, 0.75, usize::MAX);
        let ty = mau(&vao).len() as f32 / mau(&ra).len() as f32;
        assert!((ty - 0.75).abs() < 0.05, "tỉ lệ ra {ty:.3}, đáng lẽ 0.75");
        let f = hz(&ra);
        assert!((f - 220.0).abs() < 15.0, "cao độ thành {f:.0} Hz");
    }

    /// **Chốt lý do bộ giãn phải có trạng thái.** VieNeu đẩy tiếng theo mẩu; nếu cắt mẩu làm
    /// đổi kết quả thì mỗi ranh giới mẩu là một chỗ gãy — đúng tật đã sửa ở wave_out.rs.
    #[test]
    fn chia_manh_kieu_gi_cung_ra_mot_ket_qua() {
        let vao = sin(300.0, 0.6);
        let mot_lan = keo(&vao, 1.4, usize::MAX);
        for manh in [137, 1000, 4801] {
            assert_eq!(keo(&vao, 1.4, manh), mot_lan, "chia mẩu {manh} mẫu ra khác");
        }
    }

    /// Không được nuốt đuôi câu: `flush` phải trả nốt phần còn trong đệm (~50ms), nếu không
    /// mỗi câu mất phụ âm cuối.
    #[test]
    fn khong_nuot_duoi_cau() {
        let vao = sin(300.0, 0.5);
        let ra = keo(&vao, 1.0001, usize::MAX); // gần 1x -> ra phải xấp xỉ vào
        let (n_vao, n_ra) = (mau(&vao).len(), mau(&ra).len());
        assert!(n_ra * 100 > n_vao * 98, "vào {n_vao} mẫu, ra {n_ra} — đang cụt đuôi");
    }

    /// Câu ngắn hơn một cửa sổ (~32ms) vẫn phải ra tiếng, không được trả rỗng.
    #[test]
    fn cau_ngan_hon_mot_cua_so() {
        let vao = sin(300.0, 0.01);
        assert_eq!(keo(&vao, 1.5, usize::MAX).len(), vao.len());
    }

    /// Nghe thử trên TIẾNG THẬT của VieNeu, đẩy theo đúng từng mẩu như đường chạy thật (nên
    /// nghe được cả chỗ bắc cầu giữa mẩu). Sóng sin ở các test trên chứng minh được toán học
    /// nhưng không chứng minh được là nghe lọt tai.
    ///
    ///     VN_CHUNKS=<thư mục NNNN.wav> VN_OUT=<thư mục ra> \
    ///       cargo test --release --lib stretch::tests::nghe_thu -- --ignored --nocapture
    #[test]
    #[ignore]
    fn nghe_thu_tieng_that() {
        let (vao, ra) = (std::env::var("VN_CHUNKS").unwrap(), std::env::var("VN_OUT").unwrap());
        let mut manh: Vec<_> = std::fs::read_dir(&vao).unwrap().map(|e| e.unwrap().path()).collect();
        manh.sort();
        assert!(!manh.is_empty(), "không có mẩu nào trong {vao}");

        for &ty in &[0.5f32, 0.75, 1.25, 1.5, 2.0] {
            let mut s = Stretch::new(SR, ty);
            let mut pcm = Vec::new();
            for m in &manh {
                let wav = std::fs::read(m).unwrap();
                let (sr, dl) = crate::wave_out::split_wav(&wav).expect("mẩu phải là WAV");
                assert_eq!(sr, SR);
                s.push(dl, &mut pcm);
            }
            s.flush(&mut pcm);
            let f = format!("{ra}/{}x.wav", ty.to_string().replace('.', "_"));
            std::fs::write(&f, wav_hoan_chinh(&pcm)).unwrap();
            println!("{ty}x -> {:.2}s  {f}", pcm.len() as f32 / (SR as f32 * 2.0));
        }
    }

    fn wav_hoan_chinh(pcm: &[u8]) -> Vec<u8> {
        let mut w = Vec::with_capacity(44 + pcm.len());
        w.extend(b"RIFF");
        w.extend(((36 + pcm.len()) as u32).to_le_bytes());
        w.extend(b"WAVEfmt ");
        w.extend(16u32.to_le_bytes());
        w.extend(1u16.to_le_bytes()); // PCM
        w.extend(1u16.to_le_bytes()); // mono
        w.extend(SR.to_le_bytes());
        w.extend((SR * 2).to_le_bytes()); // byte/giây
        w.extend(2u16.to_le_bytes()); // khối
        w.extend(16u16.to_le_bytes()); // bit/mẫu
        w.extend(b"data");
        w.extend((pcm.len() as u32).to_le_bytes());
        w.extend_from_slice(pcm);
        w
    }

    /// App phải chạy cùng game AAA nên chỗ nào cũng cần biết giá CPU. Yêu cầu thật là bộ giãn
    /// phải theo kịp tốc độ phát; đây chốt biên rộng gấp nhiều lần.
    ///
    /// **Ngưỡng đặt theo bản DEBUG** vì `cargo test` chạy bản debug — vòng lặp số học ở debug
    /// chậm hơn release cả chục lần. Đo được: **debug 71,1 ms, release 5,6 ms** cho 1 giây
    /// tiếng, tức bản chạy thật tốn ~0,6% một nhân. Ngưỡng 200 ms bắt được hồi quy thuật toán
    /// (nới tầm dò, bỏ BUOC_DO) mà không đỏ oan.
    #[test]
    fn du_re_de_chay_cung_game() {
        let vao = sin(300.0, 1.0);
        let t = std::time::Instant::now();
        let ra = keo(&vao, 1.5, 9600); // mẩu 0,2s như VieNeu đẩy thật
        let ms = t.elapsed().as_secs_f32() * 1000.0;
        println!("giãn 1 giây tiếng: {ms:.1} ms");
        assert!(!ra.is_empty());
        assert!(ms < 200.0, "giãn 1 giây tiếng tốn {ms:.1} ms — quá đắt để chạy lúc đang chơi");
    }
}
