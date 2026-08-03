from dedup import LineDeduper


def test_same_block_not_repeated():
    d = LineDeduper(require_stable=False)
    assert d.feed("Xin chào") == ["Xin chào"]
    assert d.feed("Xin chào") == []


def test_scrolling_only_new_line():
    d = LineDeduper(require_stable=False)
    assert d.feed("dòng một\ndòng hai") == ["dòng một", "dòng hai"]
    assert d.feed("dòng hai\ndòng ba") == ["dòng ba"]  # 'dòng hai' đã đọc


def test_substring_already_spoken_in_longer():
    d = LineDeduper(require_stable=False)
    d.feed("một đêm nọ silvermane bo cho tôi số tiền lớn")
    # dòng con đã nằm trong câu dài -> không đọc lại
    assert d.feed("một đêm nọ silvermane bo cho tôi") == []


def test_two_lines_merged_into_one_skipped():
    d = LineDeduper(require_stable=False)
    d.feed("một đêm nọ silvermane bo cho tôi")
    d.feed("số tiền lớn nhất mà tôi từng thấy trong đời")
    merged = "một đêm nọ silvermane bo cho tôi số tiền lớn nhất mà tôi từng thấy trong đời"
    assert d.feed(merged) == []  # toàn nội dung đã đọc


def test_case_and_space_insensitive():
    d = LineDeduper(require_stable=False)
    d.feed("Một Đêm Nọ")
    assert d.feed("một   đêm   nọ") == []


def test_min_len_filters_noise():
    d = LineDeduper(require_stable=False, min_len=3)
    assert d.feed(".\n-\nok") == []  # toàn nhiễu ngắn


def test_stability_gate_waits_one_frame():
    d = LineDeduper(require_stable=True)
    # lần đầu xuất hiện -> chưa ổn định -> chưa đọc
    assert d.feed("câu mới xuất hiện") == []
    # frame sau vẫn còn -> ổn định -> đọc
    assert d.feed("câu mới xuất hiện") == ["câu mới xuất hiện"]


def test_new_content_after_extension_is_spoken():
    d = LineDeduper(require_stable=False)
    d.feed("hôm sau lão ngỏ lời")
    assert d.feed("một câu hoàn toàn khác biệt ở đây") == ["một câu hoàn toàn khác biệt ở đây"]


def test_fuzzy_near_duplicate_skipped():
    d = LineDeduper(require_stable=False, fuzzy_threshold=0.8)
    d.feed("Mká chỉ là súp thôi. Ông hiểu ý tôi chứ?")
    # OCR sai vài ký tự ở đầu -> vẫn coi là trùng
    assert d.feed("Máuchí là súp thôi. Ông hiểu ý tôi chứ?") == []


def test_fuzzy_distinct_lines_not_skipped():
    d = LineDeduper(require_stable=False, fuzzy_threshold=0.82)
    d.feed("Harvey, bạn hiền à, cảm ơn ông vì góc nhìn này.")
    assert d.feed("Hôm sau, lão ngỏ lời hợp tác.") == ["Hôm sau, lão ngỏ lời hợp tác."]
