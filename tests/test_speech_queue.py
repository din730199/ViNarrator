from speech_queue import SpeechQueue


def test_empty_rejected():
    q = SpeechQueue()
    assert q.enqueue("   ") is False
    assert len(q) == 0


def test_new_accepted():
    q = SpeechQueue()
    assert q.enqueue("xin chao") is True
    assert len(q) == 1


def test_consecutive_dup_rejected_after_dequeue():
    q = SpeechQueue()
    assert q.enqueue("a") is True
    assert q.try_dequeue() == "a"
    assert q.enqueue("a") is False  # dedup giu qua ca dequeue


def test_trim_dup():
    q = SpeechQueue()
    assert q.enqueue(" a ") is True
    assert q.enqueue("a") is False


def test_cap_drops_oldest():
    q = SpeechQueue(max_size=3)
    for t in ["a", "b", "c", "d"]:
        q.enqueue(t)
    assert len(q) == 3
    assert q.try_dequeue() == "b"  # "a" bi bo


def test_reset_dedup_allows_respeak():
    q = SpeechQueue()
    q.enqueue("a")
    q.try_dequeue()
    q.reset_dedup()
    assert q.enqueue("a") is True


def test_different_texts_accepted():
    q = SpeechQueue()
    assert q.enqueue("a") is True
    assert q.enqueue("b") is True
    assert len(q) == 2
