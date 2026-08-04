//! LineDeduper — chống đọc trùng cho phụ đề cuộn/từng dòng. Port từ dedup.py.
//!
//! 1. Tách theo DÒNG.
//! 2. Chỉ đọc dòng "ổn định" (đã xuất hiện ở frame trước) -> tránh đọc nửa câu đang hiện dần.
//! 3. Lọc trùng theo lịch sử dòng đã đọc + chứa-trong-nhau + gần giống.

use std::collections::{HashSet, VecDeque};

const HISTORY: usize = 40;

pub struct LineDeduper {
    min_len: usize,
    require_stable: bool,
    fuzzy_threshold: f64,
    recent: VecDeque<String>,
    recent_set: HashSet<String>,
    prev_frame: HashSet<String>,
}

fn key_of(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

impl LineDeduper {
    pub fn new(require_stable: bool, min_len: usize, fuzzy_threshold: f64) -> Self {
        Self {
            min_len,
            require_stable,
            fuzzy_threshold,
            recent: VecDeque::new(),
            recent_set: HashSet::new(),
            prev_frame: HashSet::new(),
        }
    }

    /// Trả về các dòng (giữ nguyên gốc) MỚI cần đọc.
    pub fn feed(&mut self, ocr_text: &str) -> Vec<String> {
        let pairs: Vec<(String, String)> = ocr_text
            .lines()
            .map(|l| l.trim().to_string())
            .map(|original| (key_of(&original), original))
            .filter(|(k, _)| k.chars().count() >= self.min_len)
            .collect();

        let cur_keys: HashSet<String> = pairs.iter().map(|(k, _)| k.clone()).collect();
        let mut out = Vec::new();
        for (key, original) in pairs {
            let stable = !self.require_stable
                || self.prev_frame.contains(&key)
                || self.recent_set.contains(&key);
            if !stable || self.is_dup(&key) {
                continue;
            }
            out.push(original);
            self.recent.push_back(key.clone());
            if self.recent.len() > HISTORY {
                self.recent.pop_front();
            }
            self.recent_set = self.recent.iter().cloned().collect();
        }
        self.prev_frame = cur_keys;
        out
    }

    fn is_dup(&self, key: &str) -> bool {
        if self.recent_set.contains(key) {
            return true;
        }
        // đã đọc khi nằm trong một dòng dài hơn
        if self.recent.iter().any(|r| r.contains(key)) {
            return true;
        }
        // dòng này gồm toàn nội dung đã đọc? (vd 2 dòng gộp thành 1 dòng dài)
        let mut leftover = key.to_string();
        let mut removed = false;
        for r in &self.recent {
            if !r.is_empty() && leftover.contains(r.as_str()) {
                leftover = leftover.replace(r.as_str(), " ");
                removed = true;
            }
        }
        // `removed` là bắt buộc: không trừ được gì thì leftover vẫn là câu gốc, và câu MỘT TỪ
        // ("Run!", "Help!") luôn đếm ra 1 -> bị tuyên trùng dù chưa đọc bao giờ. Bug port từ
        // bản Python, nuốt đúng loại câu cảm thán hay gặp trong game.
        if removed && leftover.split_whitespace().count() <= 1 {
            return true;
        }
        // gần giống dòng đã đọc (OCR sai vài ký tự) -> coi là trùng.
        // ponytail: difflib.SequenceMatcher (Ratcliff-Obershelp) -> normalized_levenshtein.
        // Khác metric nên ngưỡng có thể phải chỉnh; fuzzy_threshold vẫn là knob trong config.
        if self.fuzzy_threshold < 1.0 {
            return self
                .recent
                .iter()
                .any(|r| strsim::normalized_levenshtein(key, r) >= self.fuzzy_threshold);
        }
        false
    }

    pub fn reset(&mut self) {
        self.recent.clear();
        self.recent_set.clear();
        self.prev_frame.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(stable: bool) -> LineDeduper {
        LineDeduper::new(stable, 2, 0.82)
    }

    #[test]
    fn same_block_not_repeated() {
        let mut d = d(false);
        assert_eq!(d.feed("Xin chào"), ["Xin chào"]);
        assert!(d.feed("Xin chào").is_empty());
    }

    #[test]
    fn scrolling_only_new_line() {
        let mut d = d(false);
        assert_eq!(d.feed("dòng một\ndòng hai"), ["dòng một", "dòng hai"]);
        assert_eq!(d.feed("dòng hai\ndòng ba"), ["dòng ba"]);
    }

    #[test]
    fn substring_already_spoken_in_longer() {
        let mut d = d(false);
        d.feed("một đêm nọ silvermane bo cho tôi số tiền lớn");
        assert!(d.feed("một đêm nọ silvermane bo cho tôi").is_empty());
    }

    #[test]
    fn two_lines_merged_into_one_skipped() {
        let mut d = d(false);
        d.feed("một đêm nọ silvermane bo cho tôi");
        d.feed("số tiền lớn nhất mà tôi từng thấy trong đời");
        let merged = "một đêm nọ silvermane bo cho tôi số tiền lớn nhất mà tôi từng thấy trong đời";
        assert!(d.feed(merged).is_empty());
    }

    #[test]
    fn case_and_space_insensitive() {
        let mut d = d(false);
        d.feed("Một Đêm Nọ");
        assert!(d.feed("một   đêm   nọ").is_empty());
    }

    /// Ranh giới 3 là có chủ đích: bỏ mảnh chữ 1-2 ký tự nhưng GIỮ thoại 3 ký tự.
    /// (Rác dấu câu đã bị `ocr::drop_junk_lines` chặn từ trước, không tới đây.)
    #[test]
    fn do_dai_toi_thieu_mac_dinh_giu_duoc_thoai_ngan() {
        let n = crate::config::Config::default().min_line_len;
        let mut d = LineDeduper::new(false, n, 0.97);
        for real in ["Yes", "No!", "Go!", "Run", "Hey"] {
            assert_eq!(d.feed(real).len(), 1, "thoại thật bị bỏ: {real}");
        }
        // mảnh vụn OCR mà bộ lọc chữ rác không bắt được (toàn chữ cái) thì luật này bắt
        let mut d = LineDeduper::new(false, n, 0.97);
        assert!(d.feed("ll\no\nat").is_empty());
    }

    #[test]
    fn min_len_filters_noise() {
        let mut d = LineDeduper::new(false, 3, 0.82);
        assert!(d.feed(".\n-\nok").is_empty());
    }

    #[test]
    fn stability_gate_waits_one_frame() {
        let mut d = d(true);
        assert!(d.feed("câu mới xuất hiện").is_empty());
        assert_eq!(d.feed("câu mới xuất hiện"), ["câu mới xuất hiện"]);
    }

    #[test]
    fn new_content_after_extension_is_spoken() {
        let mut d = d(false);
        d.feed("hôm sau lão ngỏ lời");
        assert_eq!(
            d.feed("một câu hoàn toàn khác biệt ở đây"),
            ["một câu hoàn toàn khác biệt ở đây"]
        );
    }

    #[test]
    fn cau_mot_tu_van_duoc_doc() {
        let mut d = d(false);
        assert_eq!(d.feed("Run!"), ["Run!"]);
        assert_eq!(d.feed("Help!"), ["Help!"]);
        // nhưng lặp lại thì vẫn phải bị chặn
        assert!(d.feed("Run!").is_empty());
    }

    #[test]
    fn cau_mot_tu_khong_pha_luat_gop_dong() {
        // luật "trừ hết phần đã đọc, còn <=1 từ thì bỏ" phải còn nguyên tác dụng
        let mut d = d(false);
        d.feed("một đêm nọ silvermane bo cho tôi");
        assert!(d.feed("một đêm nọ silvermane bo cho tôi nhé").is_empty());
    }

    /// Khoá lại kết quả ĐO ĐƯỢC (60 ảnh phụ đề xuống cấp, xem README): ngưỡng 0.82 cũ nuốt
    /// mất câu thoại chỉ khác một từ — thứ tệ hơn nhiều so với đọc lặp.
    #[test]
    fn nguong_mac_dinh_khong_nuot_thoai_khac_mot_tu() {
        let default_t = crate::config::Config::default().fuzzy_threshold;
        let pairs = [
            ("i told you not to touch that lever", "i told you not to touch that button"),
            ("we need more iron before we can craft this", "we need more gold before we can craft this"),
            ("the boss spawns when the timer hits zero", "the boss despawns when the timer hits zero"),
            ("let us regroup at the base and try again", "let us regroup at the gate and try again"),
        ];
        for (a, b) in pairs {
            let mut d = LineDeduper::new(false, 4, default_t);
            assert_eq!(d.feed(a).len(), 1, "{a}");
            assert_eq!(d.feed(b).len(), 1, "câu thứ hai bị nuốt: {b}");
            // và đúng những cặp này bị nuốt ở ngưỡng cũ -> đó là lý do đổi
            let mut old = LineDeduper::new(false, 4, 0.82);
            old.feed(a);
            assert!(old.feed(b).is_empty(), "ngưỡng 0.82 lẽ ra phải nuốt: {b}");
        }
    }

    #[test]
    fn fuzzy_van_bat_duoc_ocr_lech_mot_ky_tu() {
        let default_t = crate::config::Config::default().fuzzy_threshold;
        let mut d = LineDeduper::new(false, 4, default_t);
        // OCR đọc "0" thành "o" giữa câu: không phải chuỗi con, chỉ fuzzy mới bắt được
        d.feed("the b0ss spawns when the timer hits zero at midnight");
        assert!(d.feed("the boss spawns when the timer hits zero at midnight").is_empty());
    }

    #[test]
    fn fuzzy_near_duplicate_skipped() {
        let mut d = LineDeduper::new(false, 2, 0.8);
        d.feed("Mká chỉ là súp thôi. Ông hiểu ý tôi chứ?");
        assert!(d.feed("Máuchí là súp thôi. Ông hiểu ý tôi chứ?").is_empty());
    }

    #[test]
    fn fuzzy_distinct_lines_not_skipped() {
        let mut d = d(false);
        d.feed("Harvey, bạn hiền à, cảm ơn ông vì góc nhìn này.");
        assert_eq!(
            d.feed("Hôm sau, lão ngỏ lời hợp tác."),
            ["Hôm sau, lão ngỏ lời hợp tác."]
        );
    }
}
